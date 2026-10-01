//! Shared API wire types between `cade-server` and the `cade-gui` WASM client.
//!
//! Strict rules:
//! - Pure `serde` only. No tokio / reqwest / parking_lot / native-only deps.
//! - Must compile under both `x86_64-unknown-linux-gnu` and
//!   `wasm32-unknown-unknown`. Enforced by CI target.
//! - Types mirror the JSON shapes returned by the existing `cade-server` REST
//!   endpoints and SSE streams. They are **additive**: adding fields is OK,
//!   removing or renaming is a breaking API change that requires approval.

use serde::{Deserialize, Serialize};

/// Minimal agent descriptor — what `GET /v1/agents` returns per row.
///
/// Fields marked `Option` are absent in some server responses (older rows,
/// freshly created agents). Keep them optional to stay tolerant of drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Theme name last persisted via `/theme <name>` (built-in or user theme).
    /// `None` → GUI should use the default dark theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// Response shape of `GET /v1/health`.
///
/// Mirrors the JSON returned by `cade-server` — see
/// `crates/cade-server/src/server/api/health.rs::get_health`. Fields are
/// additive; never remove or rename without bumping the wire contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthInfo {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A single message in a conversation — what `GET /v1/agents/:id/messages`
/// returns per row.
///
/// The `content` field is `serde_json::Value` because the server stores both
/// plain-text strings and structured JSON (tool calls, multi-part content).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

impl ChatMessage {
    /// Persisted turns wrap text in `{content: ...}`; older servers use strings.
    pub fn text(&self) -> String {
        let mut text = text_value(&self.content);
        if self.role == "assistant"
            && let Some(calls) = self.content.get("tool_calls").and_then(|v| v.as_array())
        {
            for call in calls {
                if let Some(name) = call.get("name").and_then(|v| v.as_str()) {
                    let args = call.get("arguments").cloned().unwrap_or_default();
                    let args = args
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| args.to_string());
                    text.push_str(&format!("\n\n[Tool call: {name}]\nArguments: {args}"));
                }
            }
        }
        text
    }
}

pub fn text_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| value.to_string())
}

/// Accept the production list envelope and the legacy bare array. A malformed
/// envelope is an error, never an empty history that can erase a live timeline.
pub fn decode_list<T: serde::de::DeserializeOwned>(
    body: &str,
    field: &str,
) -> Result<Vec<T>, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = if value.is_array() {
        value
    } else {
        value.get(field).cloned().unwrap_or(serde_json::Value::Null)
    };
    serde_json::from_value(rows)
}

/// A conversation associated with an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInfo {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

/// A single event from the server's SSE stream (`POST /v1/agents/:id/messages/stream`).
///
/// The `message_type` discriminator identifies the event kind, while `data`
/// holds all other fields via serde `flatten`:
///
/// | `message_type`        | Extra fields                                  |
/// |-----------------------|-----------------------------------------------|
/// | `stream_start`        | `conversation_id`, `run_id` (optional)        |
/// | `assistant_message`   | `content` (string, possibly incremental)      |
/// | `reasoning_message`   | `reasoning` (string)                          |
/// | `tool_call_message`   | `tool_call` `{ id, name, arguments }`         |
/// | `tool_result_message` | `tool_result` `{ id, name, output, is_error }`|
/// | `usage_statistics`    | `input_tokens`, `output_tokens`, `model`…     |
/// | `finish_reason`       | `reason` (string)                             |
/// | `error`               | `error` (string)                              |
///
/// Run events use `message_type`, questions also use `type`, and the global
/// feed uses `event_type`. Decoding normalizes these and legacy `data` envelopes
/// while serialization retains the canonical flattened `message_type` shape.
#[derive(Debug, Clone, Serialize)]
pub struct StreamEvent {
    #[serde(default)]
    pub message_type: String,
    /// Catch-all for every field other than `message_type`.
    #[serde(flatten)]
    pub data: serde_json::Value,
}

impl<'de> Deserialize<'de> for StreamEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut fields = value
            .as_object()
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("event must be an object"))?;
        let mut kind = ["message_type", "type", "event_type"]
            .iter()
            .find_map(|key| {
                fields
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("")
            .to_owned();
        // Older clients and replay envelopes may put fields inside `data`.
        if let Some(serde_json::Value::Object(data)) = fields.get("data").cloned() {
            fields.remove("data");
            for (key, value) in data {
                fields.entry(key).or_insert(value);
            }
        }
        if kind.is_empty() {
            kind = ["message_type", "type", "event_type"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_owned();
        }
        fields.remove("message_type");
        Ok(Self {
            message_type: kind,
            data: serde_json::Value::Object(fields),
        })
    }
}

impl StreamEvent {
    pub fn msg_type(&self) -> &str {
        self.message_type.as_str()
    }

    /// Extract `content` from an `assistant_message` (or any event carrying it).
    pub fn content(&self) -> Option<&str> {
        self.data.get("content").and_then(|v| v.as_str())
    }

    /// Extract `reasoning` from a `reasoning_message`.
    pub fn reasoning(&self) -> Option<&str> {
        self.data.get("reasoning").and_then(|v| v.as_str())
    }

    /// Extract the error string from an `error` event.
    pub fn error(&self) -> Option<&str> {
        self.data.get("error").and_then(|v| v.as_str())
    }

    /// Extract `tool_name` from a `tool_call_message` or `tool_result_message`.
    pub fn tool_name(&self) -> Option<&str> {
        self.tool_payload()
            .get("tool_name")
            .or_else(|| self.tool_payload().get("name"))
            .and_then(|v| v.as_str())
    }

    /// Extract `tool_args` from a `tool_call_message`.
    pub fn tool_args(&self) -> Option<&str> {
        self.tool_payload()
            .get("tool_args")
            .or_else(|| self.tool_payload().get("arguments"))
            .and_then(|v| v.as_str())
    }

    /// Extract `tool_call_id` from a `tool_call_message` / `tool_result_message`.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool_payload()
            .get("tool_call_id")
            .or_else(|| self.tool_payload().get("id"))
            .and_then(|v| v.as_str())
    }

    /// Extract `approval_id` from an `approval_required` event.
    pub fn approval_id(&self) -> Option<&str> {
        self.data
            .get("id")
            .or_else(|| self.data.get("approval_id"))
            .and_then(|v| v.as_str())
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.data.get("conversation_id").and_then(|v| v.as_str())
    }

    pub fn tool_payload(&self) -> &serde_json::Value {
        self.data
            .get("tool_call")
            .or_else(|| self.data.get("tool_result"))
            .unwrap_or(&self.data)
    }

    pub fn tool_arguments(&self) -> serde_json::Value {
        let value = self
            .tool_payload()
            .get("arguments")
            .or_else(|| self.data.get("tool_args"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        decode_json_value(value)
    }

    pub fn tool_output(&self) -> Option<&str> {
        self.tool_payload()
            .get("output")
            .or_else(|| self.tool_payload().get("content"))
            .and_then(|v| v.as_str())
    }

    /// `finish_reason` ends a provider response, not necessarily the agent run.
    pub fn is_terminal(&self) -> bool {
        self.msg_type() == "run_done"
    }

    pub fn question_request(&self) -> Option<QuestionRequest> {
        if self.msg_type() != "question_required" {
            return None;
        }
        QuestionRequest::from_pending(&self.data)
    }

    /// The canonical approval request emitted by a server-owned run.
    pub fn approval_request(&self) -> Option<ApprovalRequest<'_>> {
        ApprovalRequest::from_parts(self.msg_type(), None, &self.data)
    }

    /// Extract `run_id` from a canonical run event.
    pub fn run_id(&self) -> Option<&str> {
        self.data.get("run_id").and_then(|v| v.as_str())
    }

    /// Extract `seq_id` from a canonical run event.
    pub fn seq_id(&self) -> Option<i64> {
        self.data.get("seq_id").and_then(|v| v.as_i64())
    }

    /// Deserialize the `tool_call` object (id, name, arguments).
    pub fn tool_call(&self) -> Option<ToolCallData> {
        serde_json::from_value(self.tool_payload().clone()).ok()
    }

    /// Deserialize the `tool_result` object (id, name, output, is_error).
    pub fn tool_result(&self) -> Option<ToolResultData> {
        serde_json::from_value(self.tool_payload().clone()).ok()
    }
}

#[derive(Debug, PartialEq)]
pub struct ApprovalRequest<'a> {
    pub id: &'a str,
    pub tool_name: &'a str,
    pub arguments: &'a serde_json::Value,
    pub reason: &'a str,
}

impl<'a> ApprovalRequest<'a> {
    /// Decode the same run approval from flattened browser events and terminal
    /// messages (whose deserializer may extract `id` from the flattened data).
    pub fn from_parts(
        kind: &str,
        id: Option<&'a str>,
        data: &'a serde_json::Value,
    ) -> Option<Self> {
        if kind != "approval_required" {
            return None;
        }
        Some(Self {
            id: id.or_else(|| {
                data.get("id")
                    .or_else(|| data.get("approval_id"))
                    .and_then(|v| v.as_str())
            })?,
            tool_name: data.get("tool_name")?.as_str()?,
            arguments: data.get("arguments")?,
            reason: data.get("reason").and_then(|v| v.as_str()).unwrap_or(""),
        })
    }
}

pub fn decode_json_value(value: serde_json::Value) -> serde_json::Value {
    value
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(value)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, rename = "multiSelect", alias = "multi_select")]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionRequest {
    pub id: String,
    pub questions: Vec<Question>,
}

impl QuestionRequest {
    /// Live questions and queue rows (JSON-string arguments) share this decoder.
    pub fn from_pending(row: &serde_json::Value) -> Option<Self> {
        let arguments = decode_json_value(row.get("arguments").cloned().unwrap_or_default());
        let questions = row
            .get("questions")
            .or_else(|| arguments.get("questions"))?;
        let questions: Vec<Question> = serde_json::from_value(questions.clone()).ok()?;
        if questions.is_empty()
            || questions
                .iter()
                .any(|q| q.header.is_empty() || q.question.is_empty())
        {
            return None;
        }
        Some(Self {
            id: row.get("id")?.as_str()?.to_owned(),
            questions,
        })
    }
}

/// Transport-neutral SSE framing. Buffers bytes until a complete line, so UTF-8
/// split across network reads and CRLF split across reads are both preserved.
#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            self.line(
                String::from_utf8_lossy(&line[..end]).trim_end_matches('\r'),
                &mut frames,
            );
        }
        frames
    }

    pub fn finish(&mut self) -> Vec<String> {
        let remaining = std::mem::take(&mut self.pending);
        let mut frames = Vec::new();
        if !remaining.is_empty() {
            self.line(
                String::from_utf8_lossy(&remaining).trim_end_matches('\r'),
                &mut frames,
            );
        }
        self.line("", &mut frames);
        frames
    }

    fn line(&mut self, line: &str, frames: &mut Vec<String>) {
        if line.is_empty() {
            if !self.data.is_empty() {
                frames.push(std::mem::take(&mut self.data).join("\n"));
            }
        } else if let Some(data) = line.strip_prefix("data:") {
            self.data
                .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
        }
    }
}

/// A tool call within a `tool_call_message` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallData {
    #[serde(alias = "tool_call_id")]
    pub id: String,
    #[serde(alias = "tool_name")]
    pub name: String,
    #[serde(alias = "tool_args", deserialize_with = "deserialize_json_text")]
    pub arguments: String,
}

fn deserialize_json_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string()))
}

/// A tool result within a `tool_result_message` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultData {
    #[serde(alias = "tool_call_id")]
    pub id: String,
    #[serde(alias = "tool_name")]
    pub name: String,
    pub output: String,
    #[serde(default)]
    pub is_error: bool,
}

// region:    --- Workflows (PRD #99 / Issue #100)

/// Status of an overall workflow run or individual pipeline step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
}

impl WorkflowStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

/// A workflow step configuration item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepDef {
    pub name: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Workflow metadata descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps_count: usize,
    #[serde(default)]
    pub steps: Vec<WorkflowStepDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<WorkflowRunSummary>,
}

/// Execution record of a workflow run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub workflow_name: String,
    pub status: WorkflowStatus,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub current_step: usize,
    pub total_steps: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Real-time event emitted during a workflow run over SSE.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepEvent {
    pub run_id: String,
    pub workflow_name: String,
    pub step_index: usize,
    pub step_name: String,
    pub status: WorkflowStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_chunk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// endregion: --- Workflows

// region:    --- Swarm & Teams

/// Summary of a team member / subagent node in the swarm topology.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamMemberSummary {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub description: String,
    pub model: Option<String>,
    pub tools: String,
    pub status: String,
}

/// Summary of an agent team definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    pub max_iterations: usize,
    pub leader_model: Option<String>,
    pub members: Vec<TeamMemberSummary>,
    pub scope: String,
}

/// Aggregated swarm topology response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmTopologyResponse {
    pub teams: Vec<TeamSummary>,
    pub standalone_subagents: Vec<TeamMemberSummary>,
    pub total_nodes: usize,
}

// endregion: --- Swarm & Teams

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_production_history_and_conversation_envelopes_are_legacy_compatible() {
        let history = r#"{"messages":[{"id":"m","role":"assistant","content":{"content":"answer"},"conversation_id":"c"}],"has_more":false}"#;
        let rows = decode_list::<ChatMessage>(history, "messages").unwrap();
        assert_eq!(rows[0].text(), "answer");
        assert_eq!(
            decode_list::<ChatMessage>(&serde_json::to_string(&rows).unwrap(), "messages").unwrap(),
            rows
        );
        assert!(decode_list::<ChatMessage>(r#"{"error":"unauthorized"}"#, "messages").is_err());
        let conversations = r#"{"conversations":[{"id":"c","agent_id":"a","title":"real title","created_at":1,"updated_at":2,"message_count":3}]}"#;
        assert_eq!(
            decode_list::<ConversationInfo>(conversations, "conversations").unwrap()[0].title,
            "real title"
        );
    }

    #[test]
    fn wire_question_and_approval_decoders_accept_production_discriminators() {
        let question: StreamEvent = serde_json::from_value(serde_json::json!({"type":"question_required","id":"q-1","agent_id":"a","run_id":"r","seq_id":9,"questions":[{"header":"Scope","question":"Which scope?","multiSelect":false,"options":[{"label":"Local","description":"Current workspace"}]}]})).unwrap();
        assert_eq!(question.msg_type(), "question_required");
        assert_eq!(question.run_id(), Some("r"));
        let request = question.question_request().unwrap();
        assert_eq!(
            request.questions[0].options[0].description,
            "Current workspace"
        );
        let queue = serde_json::json!({"id":"q-1","tool_name":"ask_user_question","arguments":serde_json::json!({"questions":request.questions}).to_string()});
        assert_eq!(QuestionRequest::from_pending(&queue), Some(request));
        let approval: StreamEvent = serde_json::from_value(serde_json::json!({"event_type":"approval_required","seq":1,"data":{"id":"app-1","tool_name":"write_file","arguments":{"path":"real.txt"}}})).unwrap();
        assert_eq!(approval.approval_request().unwrap().id, "app-1");
        assert_eq!(approval.approval_request().unwrap().reason, "");
        assert_eq!(
            approval.seq_id(),
            None,
            "global feed cursors must never be run cursors"
        );
    }

    #[test]
    fn wire_tool_projections_accept_nested_and_flattened_payloads_without_losing_arguments() {
        for payload in [
            serde_json::json!({"message_type":"tool_call_message","tool_call":{"id":"tc","name":"inspect","arguments":{"path":"Cargo.toml"}}}),
            serde_json::json!({"message_type":"tool_executing","tool_call_id":"tc","tool_name":"inspect","arguments":"{\"path\":\"Cargo.toml\"}"}),
        ] {
            let event: StreamEvent = serde_json::from_value(payload).unwrap();
            assert_eq!(event.tool_name(), Some("inspect"));
            assert_eq!(event.tool_call_id(), Some("tc"));
            assert_eq!(
                event.tool_arguments(),
                serde_json::json!({"path":"Cargo.toml"})
            );
            assert_eq!(event.tool_call().unwrap().name, "inspect");
        }
        let result: StreamEvent = serde_json::from_value(serde_json::json!({"message_type":"tool_result_message","tool_result":{"tool_call_id":"tc","tool_name":"inspect","output":"actual output","is_error":false}})).unwrap();
        assert_eq!(result.tool_output(), Some("actual output"));
        assert_eq!(result.tool_result().unwrap().id, "tc");
        let persisted: ChatMessage = serde_json::from_value(serde_json::json!({"id":"m","role":"assistant","content":{"content":"","tool_calls":[{"name":"write_file","arguments":{"path":"notes.txt","content":"actual content"}}]}})).unwrap();
        assert!(
            persisted.text().contains("notes.txt") && persisted.text().contains("actual content")
        );
    }

    #[test]
    fn wire_sse_framing_handles_chunked_unicode_crlf_multiline_and_eof() {
        let wire = ": keepalive\r\nevent: message\r\ndata:{\"type\":\"question_required\",\r\ndata: \"id\":\"q-猫\",\"questions\":[]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let mut decoder = SseDecoder::default();
        let mut frames = vec![];
        for byte in wire.as_bytes() {
            frames.extend(decoder.push(&[*byte]));
        }
        frames.extend(decoder.finish());
        assert_eq!(frames.len(), 2);
        let question: StreamEvent = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(question.approval_id(), Some("q-猫"));
        assert_eq!(frames[1], "[DONE]");
        assert!(decoder.push(b": heartbeat\n\n").is_empty());
        decoder.push(b"data: {\"message_type\":\"run_done\",\"status\":\"done\"}");
        let final_frame = decoder.finish();
        assert!(
            serde_json::from_str::<StreamEvent>(&final_frame[0])
                .unwrap()
                .is_terminal()
        );
    }

    #[test]
    fn health_info_parses_server_shape() {
        // Exact shape returned by get_health() in cade-server.
        let wire = r#"{"status":"ok","server":"cade-server","version":"0.2.0"}"#;
        let h: HealthInfo = serde_json::from_str(wire).expect("parse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server.as_deref(), Some("cade-server"));
        assert_eq!(h.version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn health_info_tolerates_missing_optional_fields() {
        // Future-proof: older servers may only return `status`.
        let wire = r#"{"status":"ok"}"#;
        let h: HealthInfo = serde_json::from_str(wire).expect("parse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server, None);
        assert_eq!(h.version, None);
    }

    #[test]
    fn agent_info_round_trips_via_json() {
        // -- Fixture
        let src = AgentInfo {
            id: "agent-abc".to_string(),
            name: "Test Agent".to_string(),
            model: Some("gpt-4o".to_string()),
            provider: None,
            theme: None,
        };

        // -- Exec
        let wire = serde_json::to_string(&src).expect("serialize");
        let back: AgentInfo = serde_json::from_str(&wire).expect("deserialize");

        // -- Check
        assert_eq!(back, src);
        // `provider: None` must be omitted on the wire.
        assert!(
            !wire.contains("\"provider\""),
            "None fields must be skipped: {wire}"
        );
    }

    #[test]
    fn agent_info_parses_server_shape_without_optional_fields() {
        // Server returns agents with missing model/provider for not-yet-configured rows.
        let wire = r#"{"id":"a","name":"n"}"#;
        let a: AgentInfo = serde_json::from_str(wire).expect("tolerant parse");
        assert_eq!(a.id, "a");
        assert_eq!(a.name, "n");
        assert_eq!(a.model, None);
        assert_eq!(a.provider, None);
    }

    #[test]
    fn chat_message_parses_server_shape() {
        // Exact shape returned by GET /v1/agents/:id/messages in cade-server.
        let wire = r#"{"id":"msg-1","role":"user","content":"hello","conversation_id":"conv-1"}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("parse");
        assert_eq!(m.id, "msg-1");
        assert_eq!(m.role, "user");
        assert_eq!(m.content, serde_json::Value::String("hello".into()));
        assert_eq!(m.conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn chat_message_tolerates_missing_optional_fields() {
        let wire = r#"{"id":"m","role":"assistant","content":"hi"}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("tolerant parse");
        assert_eq!(m.id, "m");
        assert_eq!(m.role, "assistant");
        assert_eq!(m.conversation_id, None);
    }

    #[test]
    fn chat_message_content_can_be_structured_json() {
        // The server sometimes stores content as a JSON object (tool calls, etc.)
        let wire = r#"{"id":"m","role":"tool","content":{"tool":"bash","output":"ok"}}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("parse structured content");
        assert!(m.content.is_object(), "content should be a JSON object");
    }

    // -- StreamEvent

    #[test]
    fn stream_event_parses_assistant_message() {
        let wire = r#"{"message_type":"assistant_message","content":"Hello, world!"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "assistant_message");
        assert_eq!(e.content(), Some("Hello, world!"));
    }

    #[test]
    fn stream_event_parses_stream_start() {
        let wire = r#"{"message_type":"stream_start","conversation_id":"conv-1","run_id":"run-1"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "stream_start");
        assert_eq!(
            e.data.get("conversation_id").and_then(|v| v.as_str()),
            Some("conv-1")
        );
        assert_eq!(e.data.get("run_id").and_then(|v| v.as_str()), Some("run-1"));
    }

    #[test]
    fn stream_event_parses_reasoning_message() {
        let wire = r#"{"message_type":"reasoning_message","reasoning":"thinking step..."}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "reasoning_message");
        assert_eq!(e.reasoning(), Some("thinking step..."));
    }

    #[test]
    fn stream_event_parses_error() {
        let wire = r#"{"message_type":"error","error":"LLM call failed"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "error");
        assert_eq!(e.error(), Some("LLM call failed"));
    }

    #[test]
    fn stream_event_parses_tool_call() {
        let wire = r#"{"message_type":"tool_call_message","tool_call":{"id":"tc1","name":"bash","arguments":"{}"}}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "tool_call_message");
        let tc = e.data.get("tool_call").expect("tool_call present");
        assert_eq!(tc["name"].as_str(), Some("bash"));
    }

    #[test]
    fn stream_event_parses_run_envelope() {
        let wire = r#"{"message_type":"assistant_message","content":"Hello","run_id":"run-xyz","seq_id":42}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "assistant_message");
        assert_eq!(e.content(), Some("Hello"));
        assert_eq!(e.run_id(), Some("run-xyz"));
        assert_eq!(e.seq_id(), Some(42));
    }

    #[test]
    fn stream_event_defaults_message_type() {
        let wire = r#"{"some":"thing"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), ""); // default empty
        assert_eq!(e.data.get("some").and_then(|v| v.as_str()), Some("thing"));
    }

    #[test]
    fn test_workflow_models_roundtrip() {
        let run = WorkflowRunSummary {
            run_id: "run-123".to_string(),
            workflow_name: "deploy-pipeline".to_string(),
            status: WorkflowStatus::Running,
            created_at: 1724500000,
            completed_at: None,
            current_step: 1,
            total_steps: 3,
            error: None,
        };
        let json_str = serde_json::to_string(&run).expect("serialize");
        let decoded: WorkflowRunSummary = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(decoded.run_id, "run-123");
        assert_eq!(decoded.status, WorkflowStatus::Running);
        assert_eq!(decoded.status.as_str(), "running");

        let event = WorkflowStepEvent {
            run_id: "run-123".to_string(),
            workflow_name: "deploy-pipeline".to_string(),
            step_index: 0,
            step_name: "build".to_string(),
            status: WorkflowStatus::Succeeded,
            output_chunk: Some("Compiled successfully".to_string()),
            error: None,
        };
        let ev_str = serde_json::to_string(&event).expect("serialize");
        let ev_decoded: WorkflowStepEvent = serde_json::from_str(&ev_str).expect("deserialize");
        assert_eq!(ev_decoded.step_name, "build");
        assert_eq!(ev_decoded.status, WorkflowStatus::Succeeded);
    }
}

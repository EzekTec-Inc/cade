//! Responses turn lifecycle and lossless tool continuation. Calls only leave this
//! module after a terminal response validates the complete output set.
use super::{CompletionResponse, LlmToolCall, Result, StreamChunk, Value, json, parse_token_usage};
use std::collections::BTreeMap;

const CONTINUATION_PREFIX: &str = "cade:openai-responses:v1:";

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Continuation {
    before: Vec<Value>,
    after: Vec<Value>,
    item_id: Option<String>,
}

pub(crate) fn is_continuation(value: &str) -> bool {
    value.starts_with(CONTINUATION_PREFIX)
}

fn continuation(call: &LlmToolCall) -> Option<Continuation> {
    let encoded = call
        .thought_signature
        .as_deref()?
        .strip_prefix(CONTINUATION_PREFIX)?;
    serde_json::from_str(encoded).ok()
}

fn save_continuation(call: &mut LlmToolCall, state: Continuation) -> Result<()> {
    if !state.before.is_empty() || !state.after.is_empty() || state.item_id.is_some() {
        call.thought_signature = Some(format!(
            "{CONTINUATION_PREFIX}{}",
            serde_json::to_string(&state)?
        ));
    }
    Ok(())
}

pub(super) fn replay(call: &LlmToolCall, input: &mut Vec<Value>) {
    let state = continuation(call).unwrap_or_default();
    // Only reasoning belongs in this envelope. Never turn stored metadata into
    // arbitrary input messages or additional executable function calls.
    input.extend(
        state
            .before
            .into_iter()
            .filter(|item| item["type"] == "reasoning"),
    );
    let mut item = json!({
        "type": "function_call", "call_id": call.id, "name": call.name,
        "arguments": call.arguments.to_string()
    });
    if let Some(id) = state.item_id {
        item["id"] = id.into();
    }
    input.push(item);
    input.extend(
        state
            .after
            .into_iter()
            .filter(|item| item["type"] == "reasoning"),
    );
}

pub(super) fn decode(body: &Value) -> Result<CompletionResponse> {
    if body.get("error").is_some_and(|error| !error.is_null())
        || matches!(
            body["status"].as_str(),
            Some("failed" | "cancelled" | "in_progress" | "queued")
        )
    {
        return Err(crate::Error::custom(
            "Responses request did not complete successfully",
        ));
    }
    let output = body["output"]
        .as_array()
        .ok_or_else(|| crate::Error::custom("Expected Responses output array"))?;
    let mut text = String::new();
    let mut tools = Vec::new();
    let mut reasoning = Vec::new();
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => reasoning.push(item.clone()),
            Some("message") => {
                if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        if let Some(content) =
                            part["text"].as_str().or_else(|| part["refusal"].as_str())
                        {
                            text.push_str(content);
                        }
                    }
                }
            }
            Some("function_call") => {
                // An incomplete response may contain earlier completed calls,
                // but an unfinished call is never made executable by JSON repair.
                if item["status"]
                    .as_str()
                    .is_some_and(|status| status != "completed")
                    || (body["status"] == "incomplete" && item["status"] != "completed")
                {
                    return Err(crate::Error::custom("Incomplete Responses function call"));
                }
                let arguments = item["arguments"]
                    .as_str()
                    .ok_or_else(|| crate::Error::custom("Missing Responses function arguments"))?;
                let arguments: Value = serde_json::from_str(arguments)
                    .map_err(|_| crate::Error::custom("Invalid Responses function arguments"))?;
                if !arguments.is_object() {
                    return Err(crate::Error::custom(
                        "Responses function arguments must be an object",
                    ));
                }
                let id = item["call_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| crate::Error::custom("Missing Responses function call_id"))?;
                let name = item["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| crate::Error::custom("Missing Responses function name"))?;
                let mut call = LlmToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                    thought_signature: None,
                };
                save_continuation(
                    &mut call,
                    Continuation {
                        before: std::mem::take(&mut reasoning),
                        after: Vec::new(),
                        item_id: item["id"].as_str().map(String::from),
                    },
                )?;
                tools.push(call);
            }
            _ => {}
        }
    }
    if !reasoning.is_empty()
        && let Some(last) = tools.last_mut()
    {
        let mut state = continuation(last).unwrap_or_default();
        state.after = reasoning;
        save_continuation(last, state)?;
    }
    let finish_reason = if body["status"] == "incomplete" {
        body["incomplete_details"]["reason"]
            .as_str()
            .unwrap_or("incomplete")
    } else if !tools.is_empty() {
        "tool_calls"
    } else {
        "stop"
    };
    Ok(CompletionResponse {
        content: (!text.is_empty()).then_some(text),
        tool_calls: tools,
        finish_reason: finish_reason.into(),
    })
}

#[derive(Default)]
pub(super) struct StreamState {
    output: BTreeMap<usize, Value>,
    text_emitted: bool,
}

impl StreamState {
    pub(super) fn push(&mut self, event: &Value, model: &str) -> Result<Vec<StreamChunk>> {
        let mut chunks = Vec::new();
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "response.output_item.added" | "response.output_item.done" => {
                let index = event["output_index"]
                    .as_u64()
                    .ok_or_else(|| crate::Error::custom("Missing Responses output index"))?
                    as usize;
                let item = event["item"]
                    .as_object()
                    .ok_or_else(|| crate::Error::custom("Missing Responses output item"))?;
                let entry = self.output.entry(index).or_insert_with(|| json!({}));
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    // A completed opaque item is authoritative, including absent
                    // fields. Do not replay transient fields from item.added.
                    *entry = event["item"].clone();
                } else {
                    for (key, value) in item {
                        entry[key] = value.clone();
                    }
                }
                if entry["type"] == "function_call"
                    && kind == "response.output_item.added"
                    && entry.get("status").is_none()
                {
                    entry["status"] = json!("in_progress");
                } else if entry["type"] == "function_call"
                    && kind == "response.output_item.done"
                    && item.get("status").is_none()
                {
                    entry["status"] = json!("completed");
                }
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                let index = event["output_index"]
                    .as_u64()
                    .ok_or_else(|| crate::Error::custom("Missing Responses output index"))?
                    as usize;
                let entry = self
                    .output
                    .entry(index)
                    .or_insert_with(|| json!({"type":"function_call", "status":"in_progress"}));
                if kind.ends_with(".done") {
                    entry["arguments"] = event["arguments"].clone();
                } else {
                    let mut arguments = entry["arguments"].as_str().unwrap_or_default().to_owned();
                    arguments.push_str(event["delta"].as_str().unwrap_or_default());
                    entry["arguments"] = arguments.into();
                }
            }
            "response.text.delta" | "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    self.text_emitted = true;
                    chunks.push(StreamChunk::Text(text.into()));
                }
            }
            "response.reasoning.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning_summary_text.delta" => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    chunks.push(StreamChunk::Reasoning(text.into()));
                }
            }
            "response.completed" | "response.incomplete" | "response.done" => {
                let mut response = event["response"].clone();
                if !response.is_object() {
                    return Err(crate::Error::custom("Missing terminal Responses response"));
                }
                if response.get("output").is_none() {
                    response["output"] = self.output.values().cloned().collect::<Vec<_>>().into();
                }
                if kind == "response.incomplete" {
                    response["status"] = json!("incomplete");
                }
                // Decode the whole terminal output before publishing ANY call.
                let decoded = decode(&response)?;
                if !self.text_emitted
                    && let Some(text) = decoded.content
                {
                    chunks.push(StreamChunk::Text(text));
                }
                chunks.extend(decoded.tool_calls.into_iter().map(StreamChunk::ToolCall));
                chunks.push(StreamChunk::FinishReason(
                    if response["status"] == "incomplete" {
                        decoded.finish_reason
                    } else {
                        response["status"].as_str().unwrap_or("completed").into()
                    },
                ));
                if let Some(usage) = parse_token_usage(&response["usage"], model) {
                    chunks.push(StreamChunk::Usage(usage));
                }
                chunks.push(StreamChunk::Done);
                return Ok(chunks);
            }
            _ => {}
        }
        if let Some(usage) = event
            .get("usage")
            .or_else(|| event["response"].get("usage"))
            && let Some(usage) = parse_token_usage(usage, model)
        {
            chunks.push(StreamChunk::Usage(usage));
        }
        Ok(chunks)
    }
}

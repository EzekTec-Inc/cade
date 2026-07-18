use super::*;

pub(crate) fn db_row_to_llm(row: &MessageRow) -> Vec<LlmMessage> {
    match row.role.as_str() {
        // Compaction markers are DB-level sentinels only — never sent to LLM providers.
        "compaction" => vec![],
        "tool" => {
            let raw = row.content["content"].as_str().unwrap_or("");
            // Look up the per-tool char limit.  The tool_name field was added to
            // persisted tool-result content in P3-A wiring; older rows without it
            // fall back to the global TOOL_RESULT_MAX_CHARS default.
            let tool_name = row.content["tool_name"].as_str().unwrap_or("");
            let limit = tool_output_limit(tool_name);
            let content = if raw.len() > limit {
                // Find the byte offset of the limit-th char (safe multibyte boundary).
                let byte_end = raw
                    .char_indices()
                    .nth(limit)
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len());
                if byte_end < raw.len() {
                    format!(
                        "{}\n[... truncated: {} bytes total, showing first {} chars]",
                        &raw[..byte_end],
                        raw.len(),
                        limit,
                    )
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            };
            vec![LlmMessage {
                role: "tool".to_string(),
                content,
                tool_call_id: row.content["tool_call_id"].as_str().map(String::from),
                tool_calls: None,
                images: None, cache_control: None,
            }]
        }
        "assistant" => {
            // A single DB row may have both text content and tool_calls
            let text = row.content["content"].as_str().unwrap_or("").to_string();
            let tool_calls: Option<Vec<LlmToolCall>> = row.content["tool_calls"]
                .as_array()
                .filter(|arr| !arr.is_empty()) // treat [] same as absent
                .map(|arr| {
                    arr.iter()
                        .filter_map(|tc| serde_json::from_value(tc.clone()).ok())
                        .collect()
                });
            vec![LlmMessage {
                role: "assistant".to_string(),
                content: text,
                tool_call_id: None,
                tool_calls,
                images: None, cache_control: None,
            }]
        }
        _ => {
            let content = row.content["content"].as_str().unwrap_or("").to_string();
            // Reconstruct inline images stored during the original persist call.
            let images: Option<Vec<MessageImage>> = row.content["images"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect()
                })
                .filter(|v: &Vec<_>| !v.is_empty());
            vec![LlmMessage {
                role: row.role.clone(),
                content,
                tool_call_id: None,
                tool_calls: None,
                images,
                cache_control: None,
            }]
        }
    }
}

pub(crate) fn new_msg_id() -> String {
    format!("msg-{}", Uuid::new_v4())
}

pub(crate) fn persist(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    role: &str,
    content: Value,
) {
    let mut row = MessageRow {
        id: new_msg_id(),
        agent_id: agent_id.to_string(),
        conversation_id: conversation_id.map(String::from),
        role: role.to_string(),
        content,
        char_count: 0,
    };

    // Calculate char count identical to context builder
    let llm_msgs = db_row_to_llm(&row);
    let char_count: usize = llm_msgs
        .iter()
        .map(|m| {
            m.content.chars().count()
                + m.tool_calls
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|tc| tc.arguments.to_string().len())
                    .sum::<usize>()
        })
        .sum();
    row.char_count = char_count;

    if let Err(e) = sqlite::insert_message(&state.db, &row) {
        tracing::error!(
            target: "cade::persist",
            "{}",
            fmt_persist_error("insert_message", role, agent_id, conversation_id, &e)
        );
    } else {
        let payload = serde_json::json!({
            "id": row.id,
            "agent_id": row.agent_id,
            "conversation_id": row.conversation_id.clone(),
            "role": row.role.clone(),
            "content": row.content.clone(),
            "char_count": row.char_count,
        });
        crate::server::api::agents::broadcast_global_event(serde_json::json!({
            "event_type": "message_created",
            "agent_id": agent_id,
            "conversation_id": conversation_id,
            "message": payload
        }));
    }
    // Touch the conversation's updated_at so list order stays current
    if let Some(conv_id) = conversation_id
        && let Err(e) = sqlite::touch_conversation(&state.db, conv_id)
    {
        tracing::error!(
            target: "cade::persist",
            "{}",
            fmt_persist_error("touch_conversation", role, agent_id, Some(conv_id), &e)
        );
    }
}

/// Extract and validate conversation_id from request body.
/// If present and non-empty, verifies it exists in the DB.
/// Returns Ok(Some(id)) | Ok(None) | Err(response).
pub(crate) fn resolve_conversation(
    state: &AppState,
    agent_id: &str,
    body: &Value,
) -> core::result::Result<Option<String>, axum::response::Response> {
    let conv_id = body["conversation_id"].as_str().filter(|s| !s.is_empty());
    match conv_id {
        None => Ok(None),
        Some(id) => match sqlite::get_conversation(&state.db, id) {
            Ok(Some(_)) => Ok(Some(id.to_string())),
            Ok(None) => Err(err(
                axum::http::StatusCode::NOT_FOUND,
                &format!("conversation '{id}' not found for agent '{agent_id}'"),
            )),
            Err(e) => Err(err(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            )),
        },
    }
}

// -- POST /v1/agents/:id/messages  (blocking)

/// Format the error message emitted via `tracing::error!` when a
/// message-persistence call fails.  Pure helper, kept side-effect free
/// so the formatting is unit-testable without a tracing subscriber.
///
/// Output is intentionally bounded: the error is rendered with
/// `Display` (no debug dump) and PII-sensitive fields like `content`
/// are not embedded.  Only the role, agent id, and conversation id
/// (if present) appear, alongside the underlying error.
pub(crate) fn fmt_persist_error(
    op: &str,
    role: &str,
    agent_id: &str,
    conversation_id: Option<&str>,
    err: &dyn std::fmt::Display,
) -> String {
    match conversation_id {
        Some(cid) => {
            format!("persist {op} failed for role='{role}' agent='{agent_id}' conv='{cid}': {err}")
        }
        None => {
            format!("persist {op} failed for role='{role}' agent='{agent_id}' conv=<none>: {err}")
        }
    }
}

#[cfg(test)]
mod fmt_persist_error_tests {
    use super::fmt_persist_error;

    #[test]
    fn includes_op_role_and_agent() {
        let s = fmt_persist_error("insert_message", "user", "agent-1", None, &"db locked");
        assert!(s.contains("insert_message"), "missing op: {s}");
        assert!(s.contains("role='user'"), "missing role: {s}");
        assert!(s.contains("agent='agent-1'"), "missing agent: {s}");
        assert!(s.contains("db locked"), "missing underlying err: {s}");
    }

    #[test]
    fn renders_conversation_id_when_present() {
        let s = fmt_persist_error(
            "touch_conversation",
            "user",
            "agent-1",
            Some("conv-42"),
            &"io error",
        );
        assert!(s.contains("conv='conv-42'"), "expected conv id: {s}");
    }

    #[test]
    fn renders_none_marker_when_conversation_id_absent() {
        let s = fmt_persist_error("insert_message", "assistant", "a1", None, &"x");
        assert!(s.contains("conv=<none>"), "expected <none> marker: {s}");
    }

    #[test]
    fn does_not_leak_payload_or_content_keys() {
        // The message content / payload is never passed to this helper,
        // so it cannot accidentally end up in logs.  Defence-in-depth.
        let s = fmt_persist_error("insert_message", "user", "agent-1", None, &"oops");
        assert!(!s.contains("payload"), "payload must not appear: {s}");
        assert!(
            !s.to_lowercase().contains("body"),
            "body must not appear: {s}"
        );
    }
}

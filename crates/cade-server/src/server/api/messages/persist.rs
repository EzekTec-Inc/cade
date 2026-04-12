use super::*;

pub(crate) fn db_row_to_llm(row: &MessageRow) -> Vec<LlmMessage> {
    match row.role.as_str() {
        // Compaction markers are DB-level sentinels only — never sent to LLM providers.
        "compaction" => vec![],
        "tool" => {
            let raw = row.content["content"].as_str().unwrap_or("");
            // Truncate very large tool results (e.g. raw base64 images, enormous logs)
            // to prevent context window overflows.
            // Truncate at a char boundary, not a byte boundary.
            // TOOL_RESULT_MAX_CHARS is a *char* limit; raw.len() is bytes.
            // Slicing `&raw[..N]` at a bare byte index panics when a multibyte
            // codepoint (e.g. '─' = 3 bytes: E2 94 80) straddles position N.
            let content = if raw.len() > TOOL_RESULT_MAX_CHARS {
                // Find the byte offset of the TOOL_RESULT_MAX_CHARS-th char.
                let byte_end = raw
                    .char_indices()
                    .nth(TOOL_RESULT_MAX_CHARS)
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len()); // fewer chars than the limit → keep all
                if byte_end < raw.len() {
                    format!(
                        "{}\n[... truncated: {} bytes total, showing first {} chars]",
                        &raw[..byte_end],
                        raw.len(),
                        TOOL_RESULT_MAX_CHARS,
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
                images: None,
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
                images: None,
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

    let _ = sqlite::insert_message(&state.db, &row);
    // Touch the conversation's updated_at so list order stays current
    if let Some(conv_id) = conversation_id {
        let _ = sqlite::touch_conversation(&state.db, conv_id);
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

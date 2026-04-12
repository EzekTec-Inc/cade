//! Background memory consolidation — the "Sleeptime Agent".
//!
//! When the budget-based context builder in `build_context()` drops older turns
//! from the LLM prompt it sets `needs_consolidation = true` in `agent_activity`.
//! After 60 s of agent inactivity the Sleeptime background task calls
//! [`consolidate_agent`], which summarises the dropped turns into a persistent
//! `session_summary` memory block so the agent retains the gist of past work
//! across context rotations.

use cade_ai::{CompletionRequest, LlmMessage, catalogue};

use crate::server::state::AppState;
use cade_store::sqlite;

// ── tunables ──────────────────────────────────────────────────────────────────

/// Minimum number of DB rows required before consolidation is attempted.
/// Below this the conversation is too short to be worth summarising.
const MIN_ROWS_FOR_CONSOLIDATION: usize = 20;

/// Maximum chars of formatted history text fed to the summarisation LLM call.
/// ~8 k tokens at 3 chars/token — enough context without blowing cost.
const MAX_SUMMARY_INPUT_CHARS: usize = 24_000;

/// Maximum tokens the summarisation LLM is allowed to emit.
const SUMMARY_MAX_TOKENS: u32 = 800;

/// Maximum chars stored in the `session_summary` memory block.
const SESSION_SUMMARY_MAX_CHARS: usize = 4_000;

/// Fraction of the estimated history budget used as the threshold: turns that
/// fit within `char_budget * HISTORY_BUDGET_FRACTION` are considered "in
/// context"; everything older is considered "dropped" and summarised.
const HISTORY_BUDGET_FRACTION: f64 = 0.40;

/// Characters per token approximation (conservative).
const CHARS_PER_TOKEN: usize = 3;

// ── public API ────────────────────────────────────────────────────────────────

/// Summarise older conversation turns that are no longer in the active context
/// window and write the result to the agent's `session_summary` memory block.
///
/// This is safe to call concurrently for different agents; all DB access is
/// through the existing `Arc<Mutex<Connection>>` pool.
pub async fn consolidate_agent(state: &AppState, agent_id: &str, conversation_id: Option<&str>) {
    let agent = match sqlite::get_agent(&state.db, agent_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            tracing::warn!("consolidate [{}]: agent not found — skipping", agent_id);
            return;
        }
        Err(e) => {
            tracing::warn!("consolidate [{}]: DB error: {}", agent_id, e);
            return;
        }
    };

    // ── 1. Fetch recent messages ──────────────────────────────────────────────
    let all_rows = sqlite::list_messages_page(&state.db, agent_id, conversation_id, 500, 0)
        .unwrap_or_default();

    if all_rows.len() < MIN_ROWS_FOR_CONSOLIDATION {
        tracing::debug!(
            "consolidate [{}]: only {} rows — skipping",
            agent_id,
            all_rows.len()
        );
        return;
    }

    // Convert rows to (role, text) pairs for turn grouping.
    let flat: Vec<(String, String)> = all_rows
        .iter()
        .map(|row| {
            let role = row.role.clone();
            let text = row.content["content"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| {
                    let raw = row.content.to_string();
                    if raw.len() > 400 {
                        format!("{}…", &raw[..400])
                    } else {
                        raw
                    }
                });
            (role, text)
        })
        .collect();

    // ── 2. Determine which turns are "in context" vs "dropped" ───────────────
    let window_tokens = catalogue::context_window_for_model(&agent.model) as usize;
    let output_reserve = ((window_tokens as f64) * 0.15).round() as usize;
    let input_tokens = window_tokens.saturating_sub(output_reserve);
    let char_budget = (input_tokens * CHARS_PER_TOKEN).clamp(8_000, 6_000_000);
    let history_budget = (char_budget as f64 * HISTORY_BUDGET_FRACTION).round() as usize;

    let turns = group_turns(&flat);
    let total_turns = turns.len();

    let mut in_context = 0usize;
    let mut used = 0usize;
    for turn in turns.iter().rev() {
        let chars: usize = turn.iter().map(|(_, t)| t.chars().count()).sum();
        if used + chars <= history_budget {
            in_context += 1;
            used += chars;
        } else {
            break;
        }
    }

    let dropped = total_turns.saturating_sub(in_context);
    if dropped == 0 {
        tracing::debug!(
            "consolidate [{}]: all {} turns fit in budget — nothing to summarise",
            agent_id,
            total_turns
        );
        return;
    }

    // ── 3. Format dropped turns into a text block for the LLM ────────────────
    let mut history_text = String::new();
    'outer: for turn in &turns[..dropped] {
        for (role, text) in turn {
            if history_text.chars().count() >= MAX_SUMMARY_INPUT_CHARS {
                history_text.push_str("\n[...older history truncated...]");
                break 'outer;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip noisy short tool results (already processed, low signal)
            if role == "tool" && trimmed.len() < 15 {
                continue;
            }
            // Truncate very long individual messages (file dumps, base64, etc.)
            let preview: String = if trimmed.chars().count() > 600 {
                format!("{}…", trimmed.chars().take(600).collect::<String>())
            } else {
                trimmed.to_string()
            };
            history_text.push_str(&format!("[{role}] {preview}\n"));
        }
    }

    if history_text.trim().is_empty() {
        tracing::debug!(
            "consolidate [{}]: dropped turns have no useful text — skipping",
            agent_id
        );
        return;
    }

    // ── 4. Call the LLM to produce a consolidation summary ───────────────────
    let prompt = format!(
        "You are a memory consolidation sub-agent for a stateful coding assistant.\n\
         The following is older conversation history that has scrolled out of the \
         agent's active context window.\n\
         \n\
         Extract only what the agent needs to remember for future turns:\n\
         1. The main task or goal being worked on\n\
         2. Files read, created, or modified (be specific)\n\
         3. Key decisions or approaches chosen and why\n\
         4. Problems encountered and how they were resolved\n\
         5. Work completed vs work still in progress\n\
         6. Any conventions, constraints, or preferences discovered\n\
         \n\
         Write as a concise structured note (max 400 words). Be factual and specific. \
         Do not describe the conversation format or refer to 'the user said'. \
         Write in past tense from the perspective of what happened.\n\
         \n\
         HISTORY:\n\
         {history_text}"
    );

    // Use the compaction model if configured, otherwise fall back to the main model.
    let compaction_model = agent
        .compaction_model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(&agent.model);

    let req = CompletionRequest {
        model: compaction_model.to_string(),
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: SUMMARY_MAX_TOKENS,
        reasoning_effort: None,
    };

    let summary = match state.llm.complete(&req).await {
        Ok(resp) => resp.content.unwrap_or_default().trim().to_string(),
        Err(e) => {
            tracing::warn!("consolidate [{}]: LLM call failed: {}", agent_id, e);
            return;
        }
    };

    if summary.is_empty() {
        tracing::debug!("consolidate [{}]: LLM returned empty summary", agent_id);
        return;
    }

    // ── 4b. Inflation guard — reject summary if it's larger than the dropped content ──
    let dropped_chars = history_text.chars().count();
    let summary_chars = summary.chars().count();
    if is_summary_inflated(summary_chars, dropped_chars) {
        tracing::warn!(
            "consolidate [{}]: summary inflated ({} chars) vs dropped ({} chars) — skipping",
            agent_id,
            summary_chars,
            dropped_chars,
        );
        return;
    }

    // ── 5. Write to the `session_summary` memory block ───────────────────────
    // Append to the existing summary or start fresh; cap at SESSION_SUMMARY_MAX_CHARS.
    let existing_blocks = sqlite::get_memory_blocks(&state.db, agent_id).unwrap_or_default();
    let existing = existing_blocks
        .iter()
        .find(|(label, _, _)| label == "session_summary")
        .map(|(_, val, _)| val.as_str())
        .unwrap_or("");

    let new_value = if existing.is_empty() {
        summary.clone()
    } else {
        let combined = format!("{existing}\n\n---\n\n{summary}");
        if combined.chars().count() > SESSION_SUMMARY_MAX_CHARS {
            // Combined too long — keep only the latest summary to stay useful.
            summary.clone()
        } else {
            combined
        }
    };

    if let Err(e) = sqlite::upsert_memory_block(
        &state.db,
        agent_id,
        "session_summary",
        &new_value,
        Some("Auto-generated summary of older conversation turns (Sleeptime consolidation)"),
        Some(SESSION_SUMMARY_MAX_CHARS),
    ) {
        tracing::warn!(
            "consolidate [{}]: failed to write session_summary: {}",
            agent_id,
            e
        );
        return;
    }

    tracing::info!(
        "consolidate [{}]: session_summary updated ({} chars; {} dropped turns summarised)",
        agent_id,
        new_value.chars().count(),
        dropped,
    );

    // ── 6. Insert a compaction marker into message history ───────────────────
    // The marker acts as a boundary: `get_context_window()` only loads messages
    // after the most recent marker, drastically reducing the scan set.
    //
    // Use the newest dropped message's ID to anchor the marker's timestamp.
    // The dropped turns are all_rows[0..dropped_msg_count] (oldest-first).
    // We want the marker's created_at to be equal to the newest dropped message.
    let dropped_msg_count: usize = turns[..dropped]
        .iter()
        .map(|t| t.len())
        .sum();

    // Look up the created_at of the boundary message from the DB.
    // all_rows is oldest-first; the boundary is at index dropped_msg_count - 1.
    let boundary_msg_id = if dropped_msg_count > 0 && dropped_msg_count <= all_rows.len() {
        Some(all_rows[dropped_msg_count - 1].id.clone())
    } else {
        None
    };

    if let Some(ref bid) = boundary_msg_id {
        let marker_ts = {
            let conn = state.db.lock().map_err(|e| {
                tracing::warn!("consolidate [{}]: DB lock: {}", agent_id, e);
            });
            match conn {
                Ok(c) => c
                    .query_row(
                        "SELECT created_at FROM messages WHERE id = ?1",
                        rusqlite::params![bid],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or_else(|_| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64
                    }),
                Err(_) => return,
            }
        };

        let marker_content = serde_json::json!({
            "content": format!(
                "[Compaction marker: {} turns summarised into session_summary]",
                dropped,
            ),
        });

        let marker = sqlite::MessageRow {
            id: format!("compact-{}", uuid::Uuid::new_v4()),
            agent_id: agent_id.to_string(),
            conversation_id: conversation_id.map(String::from),
            role: "compaction".to_string(),
            content: marker_content,
            char_count: 0,
        };

        // Insert with the boundary timestamp so ordering is correct.
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    marker.id,
                    marker.agent_id,
                    marker.conversation_id,
                    marker.role,
                    marker.content.to_string(),
                    marker_ts,
                    0i64,
                ],
            );
            tracing::debug!(
                "consolidate [{}]: inserted compaction marker '{}' at ts={}",
                agent_id,
                marker.id,
                marker_ts,
            );
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` if the summary is inflated relative to the source text — i.e.,
/// the summary is ≥ 80% of the dropped-content size and should be rejected.
fn is_summary_inflated(summary_chars: usize, dropped_chars: usize) -> bool {
    dropped_chars > 0 && summary_chars > ((dropped_chars as f64) * 0.8) as usize
}

/// Group `(role, text)` pairs into logical turns.
/// A turn starts at each `user` message and includes all following non-`user`
/// messages (assistant, tool) until the next `user` message.
fn group_turns(messages: &[(String, String)]) -> Vec<Vec<(String, String)>> {
    let mut turns: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    for msg in messages {
        if msg.0 == "user" && !current.is_empty() {
            turns.push(std::mem::take(&mut current));
        }
        current.push(msg.clone());
    }
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: &str, text: &str) -> (String, String) {
        (role.to_string(), text.to_string())
    }

    #[test]
    fn empty_produces_no_turns() {
        assert!(group_turns(&[]).is_empty());
    }

    #[test]
    fn single_user_message_is_one_turn() {
        let turns = group_turns(&[m("user", "hello")]);
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn two_exchanges_produce_two_turns() {
        let msgs = vec![
            m("user", "q1"),
            m("assistant", "a1"),
            m("user", "q2"),
            m("assistant", "a2"),
        ];
        let turns = group_turns(&msgs);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0][0].0, "user");
        assert_eq!(turns[1][0].0, "user");
    }

    #[test]
    fn tool_result_stays_in_same_turn_as_its_call() {
        let msgs = vec![
            m("user", "do thing"),
            m("assistant", ""), // triggers tool call
            m("tool", "ok"),    // result
            m("assistant", "done"),
            m("user", "next"),
        ];
        let turns = group_turns(&msgs);
        assert_eq!(turns.len(), 2);
        // First turn: user + assistant + tool + assistant = 4 messages
        assert_eq!(turns[0].len(), 4);
    }

    // ── Inflation guard tests ─────────────────────────────────────────────

    #[test]
    fn inflation_guard_rejects_when_summary_is_large() {
        assert!(is_summary_inflated(900, 1000));
    }

    #[test]
    fn inflation_guard_accepts_when_summary_is_compact() {
        assert!(!is_summary_inflated(200, 1000));
    }

    #[test]
    fn inflation_guard_boundary_at_80_percent() {
        assert!(!is_summary_inflated(800, 1000));
        assert!(is_summary_inflated(801, 1000));
    }

    #[test]
    fn inflation_guard_handles_zero_dropped() {
        assert!(!is_summary_inflated(100, 0));
    }

    #[test]
    fn inflation_guard_handles_empty_summary() {
        assert!(!is_summary_inflated(0, 1000));
    }
}

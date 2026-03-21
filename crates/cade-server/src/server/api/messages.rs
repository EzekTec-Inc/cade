use axum::response::sse::Event;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::{
    state::AppState,
    storage::sqlite::{self, MessageRow},
};
use cade_ai::catalogue;
use cade_ai::{CompletionRequest, LlmMessage, LlmToolCall, MessageImage, StreamChunk, TokenUsage};

/// Maximum length for auto-generated conversation titles (chars from first user message).
const CONV_TITLE_MAX: usize = 60;
/// Appended to every agent's system prompt so the LLM always produces
/// plain-text analysis after tool use, regardless of the stored system_prompt.
const TOOL_RESPONSE_RULE: &str = "\n\n\
After every tool execution, always provide a plain-text response that explains \
the result, what you found, or what you are doing next. \
Never end a turn silently after running a tool.";
/// Page size for history fetch; pages accumulate until soft budget is reached.
const HISTORY_PAGE_SIZE: usize = 100;
/// Number of messages from the end of history considered "recent".
/// Tool results inside this window are kept at full fidelity.
const RECENT_WINDOW: usize = 40;
/// Tool results outside the recent window are trimmed to this many chars.
/// They have already been processed; re-sending verbatim wastes tokens.
const STALE_TOOL_RESULT_MAX_CHARS: usize = 300;
/// Character budget for pinned memory blocks (always injected, highest priority).
const PINNED_BUDGET: usize = 2_000;
/// Character budget for short-term active memory blocks (full fidelity).
const SHORT_BUDGET: usize = 4_500;
/// Character budget for the long-term archived index (label + 80-char excerpt).
const LONG_BUDGET: usize = 1_000;
/// Turns of inactivity before a short-term block is promoted to long-term.
const STALE_THRESHOLD: i64 = 40;
/// Awareness footer appended to system prompt when any memory tier is present.
const MEMORY_AWARENESS_FOOTER: &str = "\n\nMemory system: blocks idle for 40+ turns are \
archived. The Archived Memory section above lists them with excerpts. Call \
search_memory(query) when a task may need archived context — retrieved blocks \
return to active memory automatically. Pin critical reference blocks with \
update_memory(label, value, tier=\"pinned\").";
/// Cap on a single tool-result content string (chars). ~2k tokens.
/// Prevents huge outputs (screenshots, logs) from blowing the context window.
/// 8 192 chars covers the vast majority of useful tool outputs (diffs, file
/// excerpts, command output) while cutting worst-case cost by 75% vs 32 768.
const TOOL_RESULT_MAX_CHARS: usize = 8_192;
/// Chars-per-token approximation used to convert a model's token context window
/// into a character budget.  The budget formula is:
///   char_budget = input_budget_tokens × CHARS_PER_TOKEN
/// A LOWER value is more conservative (allows fewer chars per allocated token).
/// 3 chars/token with typical 3.5–4 c/t text yields ~15–25% headroom below the
/// hard token limit, preventing accidental overflow.
const CHARS_PER_TOKEN: usize = 3;
/// Minimum character budget regardless of model window (guards tiny local models).
const MIN_CONTEXT_CHARS: usize = 8_000;
/// Maximum character budget cap.  6_000_000 chars ≈ 2 M tokens at 3 chars/token,
/// which fully covers Gemini 2 M.  Claude 200 K is unaffected
/// (200_000 × 3 = 600_000 < cap).
const MAX_CONTEXT_CHARS: usize = 6_000_000;

/// Fraction of the context window reserved for the model's output (including
/// reasoning/thinking tokens).  0.15 means 15% of the total window is off-limits
/// to input context.  For a 128k model this reserves ~19k tokens for output,
/// which is enough for max_tokens (8192) + reasoning budget (up to 16k).
const OUTPUT_RESERVE_FRACTION: f64 = 0.15;

/// Estimated per-tool schema overhead in characters.  Used to subtract tool
/// schema cost from the context budget *before* filling with message history.
/// A typical tool schema is ~400–800 chars; 600 is a reasonable average.
const TOOL_SCHEMA_CHARS_ESTIMATE: usize = 600;

// -- Auto-compaction constants

/// Context usage ratio at which auto-compaction (summarization) triggers.
/// 0.85 means: when the assembled messages use ≥ 85% of the (already-reserved)
/// char budget.  Triggering earlier than the old 0.98 gives the summarizer
/// room to run without the hard-trim loop immediately evicting the summary.
const COMPACT_THRESHOLD: f64 = 0.85;
/// Emergency compaction threshold — bypasses cooldown.  When context hits this
/// level, compaction runs regardless of how recently the last one happened.
const COMPACT_EMERGENCY_THRESHOLD: f64 = 0.95;
/// Minimum number of user+assistant messages that must exist before
/// compaction is even considered (avoids summarizing trivially short sessions).
const COMPACT_MIN_MESSAGES: usize = 10;
/// Number of recent messages to keep at full fidelity (never summarized).
/// Must be ≥ 4 so the model always sees the latest user+assistant exchange.
const COMPACT_KEEP_RECENT: usize = 8;
/// Cooldown: minimum number of turns between successive compactions for
/// the same agent.  Prevents re-summarizing every turn once the threshold
/// is crossed.  Bypassed when usage ≥ COMPACT_EMERGENCY_THRESHOLD.
const COMPACT_COOLDOWN_TURNS: i64 = 5;

// -- Message history sanitizer
//
// Anthropic enforces a strict schema:
//   1. Every tool_use in an assistant message must have exactly ONE matching
//      tool_result in the very next user message.
//   2. No tool_result may appear without a preceding tool_use.
//   3. No duplicate tool_result IDs in the same user message.
//
// Previous bugs (SSE reconnect, interrupted sessions) can leave the DB in a
// state that violates these rules.  sanitize_messages() repairs the history
// before it is sent to the LLM so the server never crashes due to DB
// corruption.

fn sanitize_messages(messages: Vec<LlmMessage>) -> Vec<LlmMessage> {
    let mut result: Vec<LlmMessage> = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = messages[i].clone();

        match msg.role.as_str() {
            "assistant" if msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                let tool_calls = msg.tool_calls.as_deref().unwrap_or_default();
                let expected_ids: Vec<String> = tool_calls.iter().map(|tc| tc.id.clone()).collect();

                // Consume ALL immediately-following tool rows (may be duplicated/partial)
                let mut j = i + 1;
                // id → first content seen (dedup)
                let mut found: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                while j < messages.len() && messages[j].role == "tool" {
                    if let Some(id) = &messages[j].tool_call_id {
                        found
                            .entry(id.clone())
                            .or_insert_with(|| messages[j].content.clone());
                    }
                    j += 1;
                }

                result.push(msg);

                // Emit exactly one tool_result per expected id (synthetic if missing)
                for id in &expected_ids {
                    let content = found
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "[Tool execution was interrupted]".to_string());
                    result.push(LlmMessage {
                        role: "tool".to_string(),
                        content,
                        tool_call_id: Some(id.clone()),
                        tool_calls: None,
                        images: None,
                    });
                }

                i = j; // skip the (possibly messy) original tool rows
            }

            "tool" => {
                // Orphaned tool_result — no preceding assistant with a matching tool_use.
                // Drop it; it would make Anthropic return 400.
                tracing::warn!("Dropping orphaned tool_result (id={:?})", msg.tool_call_id);
                i += 1;
            }

            _ => {
                result.push(msg);
                i += 1;
            }
        }
    }

    result
}

// -- Auto-compaction: summarize old turns into short-term memory

/// Summarize a slice of conversation messages into a compact block.
///
/// Uses the same LLM provider that the agent normally talks to.
/// Returns `Ok(summary_text)` on success, or an error (caller should
/// log and fall back to plain trimming).
async fn summarize_for_compaction(
    state: &AppState,
    model: &str,
    chunk: &[LlmMessage],
) -> core::result::Result<String, String> {
    // Format the chunk as a readable transcript for the summarizer.
    let mut transcript = String::new();
    for msg in chunk {
        if msg.role == "system" {
            continue;
        }
        let role_label = match msg.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            "tool" => "Tool result",
            _ => &msg.role,
        };
        if !msg.content.is_empty() {
            transcript.push_str(&format!("[{role_label}]: {}\n\n", msg.content));
        }
        if let Some(calls) = &msg.tool_calls {
            for tc in calls {
                transcript.push_str(&format!(
                    "[{role_label} called {}({})]\n\n",
                    tc.name,
                    serde_json::to_string(&tc.arguments).unwrap_or_default()
                ));
            }
        }
    }

    if transcript.trim().is_empty() {
        return Err("empty transcript — nothing to summarize".to_string());
    }

    // Cap the transcript we send to the summarizer to avoid exceeding its own
    // context window.  Use at most ~40% of the model's budget for the input.
    let model_budget = {
        let w = catalogue::context_window_for_model(model) as usize;
        (w * CHARS_PER_TOKEN).clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS)
    };
    let max_input_chars = model_budget * 2 / 5;
    if transcript.chars().count() > max_input_chars {
        let byte_end = transcript
            .char_indices()
            .nth(max_input_chars)
            .map(|(i, _)| i)
            .unwrap_or(transcript.len());
        transcript.truncate(byte_end);
        transcript.push_str("\n[…transcript truncated for summarization]");
    }

    let system_msg = LlmMessage {
        role: "system".to_string(),
        content: "You are a precise summarizer. Produce a concise summary of the \
conversation below. Preserve: task goals, key decisions, file paths, \
code changes, constraints, and current state. Omit: greetings, filler, \
verbose tool output. Output plain text, no markdown headers. \
Keep under 800 words."
            .to_string(),
        tool_call_id: None,
        tool_calls: None,
        images: None,
    };
    let user_msg = LlmMessage {
        role: "user".to_string(),
        content: format!("Summarize this conversation:\n\n{transcript}"),
        tool_call_id: None,
        tool_calls: None,
        images: None,
    };

    let req = CompletionRequest {
        model: model.to_string(),
        messages: vec![system_msg, user_msg],
        tools: vec![],
        max_tokens: 2048,
        reasoning_effort: None,
    };

    let resp = state.llm.complete(&req).await.map_err(|e| e.to_string())?;
    resp.content
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "summarizer returned empty content".to_string())
}

// -- Context builder
//
// Key design rule:
//   Callers PERSIST a message to SQLite BEFORE calling build_context.
//   build_context loads everything from SQLite — no new_message parameter.
//   This prevents the double-message bug that breaks tool_use/tool_result ordering.

async fn build_context(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    is_tool_return: bool,
) -> core::result::Result<(String, Vec<LlmMessage>, Vec<Value>), String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    // -- Three-tier memory injection
    //
    // Tiers:  pinned (always, full)  |  short (active, full)  |  long (archived, excerpt)
    //
    // Turn counter increments once per user message (not per tool return) so
    // "20 turns idle" means 20 real user↔agent exchanges, not 20 tool calls.

    // 1. Advance (or read) the turn counter.
    let current_turn = if is_tool_return {
        sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0)
    } else {
        sqlite::increment_turn_counter(&state.db, agent_id).unwrap_or(0)
    };

    // 2. Promote stale short blocks to long.
    let _ = sqlite::promote_stale_blocks(&state.db, agent_id, current_turn, STALE_THRESHOLD);

    // 3. Pinned + short-term blocks → full value, greedy-packed into budgets.
    let active_blocks = sqlite::get_active_blocks(&state.db, agent_id).unwrap_or_default();
    let mut pinned_parts: Vec<String> = Vec::new();
    let mut short_parts: Vec<String> = Vec::new();
    let mut pinned_remaining = PINNED_BUDGET;
    let mut short_remaining = SHORT_BUDGET;
    let mut active_omitted = 0usize;

    for (label, val, _desc, tier, _lt) in &active_blocks {
        if val.trim().is_empty() {
            continue;
        }
        if tier == "pinned" {
            let entry = format!("📌 [{label}]\n{val}");
            let chars = entry.chars().count();
            if chars <= pinned_remaining {
                pinned_remaining -= chars;
                pinned_parts.push(entry);
            } else {
                active_omitted += 1;
            }
        } else {
            let entry = format!("[{label}]\n{val}");
            let chars = entry.chars().count();
            if chars <= short_remaining {
                short_remaining -= chars;
                short_parts.push(entry);
            } else {
                active_omitted += 1;
            }
        }
    }

    // 4. Long-term archived blocks → label + excerpt only.
    let long_excerpts =
        sqlite::get_long_term_excerpts(&state.db, agent_id, current_turn).unwrap_or_default();
    let mut long_parts: Vec<String> = Vec::new();
    let mut long_remaining = LONG_BUDGET;
    let mut long_omitted = 0usize;

    for (label, excerpt, _idle) in &long_excerpts {
        let entry = if excerpt.trim().is_empty() {
            format!("[{label}]")
        } else {
            format!("[{label}]: {excerpt}")
        };
        let chars = entry.chars().count();
        if chars <= long_remaining {
            long_remaining -= chars;
            long_parts.push(entry);
        } else {
            long_omitted += 1;
        }
    }

    // 5. Assemble system prompt memory sections.
    let has_any_memory =
        !pinned_parts.is_empty() || !short_parts.is_empty() || !long_parts.is_empty();
    let base = agent.system_prompt.clone().unwrap_or_default();

    let system_core = if !has_any_memory {
        base
    } else {
        let mut sections: Vec<String> = vec![base];

        // Active memory section (pinned + short)
        let mut active_section_parts: Vec<String> = Vec::new();
        active_section_parts.extend(pinned_parts);
        active_section_parts.extend(short_parts);
        if active_omitted > 0 {
            active_section_parts.push(format!(
                "[…{active_omitted} block(s) omitted — memory budget reached. Use /memory to manage.]"
            ));
        }
        if !active_section_parts.is_empty() {
            sections.push(format!("# Memory\n{}", active_section_parts.join("\n\n")));
        }

        // Archived memory section (long-term excerpts)
        if !long_parts.is_empty() {
            let mut archived = long_parts.join("\n");
            if long_omitted > 0 {
                archived.push_str(&format!(
                    "\n[…{long_omitted} more archived — use /memory or search_memory]"
                ));
            }
            sections.push(format!(
                "# Archived Memory\n{archived}\nUse search_memory(query) to retrieve full archived content.\nAccessed blocks are automatically restored to active memory."
            ));
        }

        // Append awareness footer when any memory exists
        let mut core = sections.join("\n\n");
        core.push_str(MEMORY_AWARENESS_FOOTER);
        core
    };
    // Memory-change detection: cache the assembled system_core per agent.
    // If the content hash matches the last cached value the string is identical
    // to the previous turn, so the LLM provider's implicit prompt cache
    // (OpenAI KV cache, Gemini implicit cache) is guaranteed to hit.
    let system_prompt = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        system_core.hash(&mut h);
        let new_hash = h.finish();
        let mut cache = state.memory_cache.lock().unwrap_or_else(|e| e.into_inner());
        let entry = cache
            .entry(agent_id.to_string())
            .or_insert((0, String::new()));
        if entry.0 != new_hash {
            entry.0 = new_hash;
            entry.1 = system_core;
        }
        format!("{}{TOOL_RESPONSE_RULE}", entry.1)
    };

    // Message history from DB — oldest first, scoped to conversation
    let mut messages: Vec<LlmMessage> = vec![LlmMessage {
        role: "system".to_string(),
        content: system_prompt,
        tool_call_id: None,
        tool_calls: None,
        images: None,
    }];

    // Character-budget trimming — reserves space for output tokens, reasoning
    // tokens, and tool schemas so the model has room to generate a full response.
    //
    //  total_window  = context_window_for_model (e.g. 128k tokens)
    //  output_reserve = total_window × OUTPUT_RESERVE_FRACTION  (e.g. 15% = 19.2k)
    //  input_budget   = total_window - output_reserve            (e.g. 108.8k)
    //  char_budget    = input_budget × CHARS_PER_TOKEN           (e.g. 435k chars)
    //  tool_reserve   = n_tools × TOOL_SCHEMA_CHARS_ESTIMATE     (subtracted below)
    //  message_budget = char_budget - tool_reserve
    let window_tokens = catalogue::context_window_for_model(&agent.model);
    let output_reserve_tokens = ((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize;
    let input_budget_tokens = (window_tokens as usize).saturating_sub(output_reserve_tokens);
    let context_char_budget = {
        let raw = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
        raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS)
    };
    // Estimate tool-schema overhead and subtract from the message budget.
    // Tool schemas are loaded at the end of build_context, but their token cost
    // must be reserved up-front so we don't fill the window with history and then
    // overflow when schemas are appended to the LLM request.
    let agent_tool_count = sqlite::get_agent_tool_ids(&state.db, agent_id)
        .unwrap_or_default()
        .len()
        .max(1); // at least 1 — even with no wired tools, meta tools are always sent
    let tool_schema_reserve = agent_tool_count * TOOL_SCHEMA_CHARS_ESTIMATE;
    let context_char_budget = context_char_budget.saturating_sub(tool_schema_reserve);
    let context_char_budget = context_char_budget.max(MIN_CONTEXT_CHARS);
    tracing::debug!(
        "Context budget for model '{}': {} chars (window={} tokens, output_reserve={}, \
         input={} × {}, tool_reserve={}×{}={} chars)",
        agent.model,
        context_char_budget,
        window_tokens,
        output_reserve_tokens,
        input_budget_tokens,
        CHARS_PER_TOKEN,
        agent_tool_count,
        TOOL_SCHEMA_CHARS_ESTIMATE,
        tool_schema_reserve,
    );
    // Count both content text AND tool_calls JSON so tool-heavy sessions are
    // trimmed accurately.  Counting only content underestimates context size
    // when many tool-call schemas / large argument payloads are in history.
    let total_chars = |msgs: &[LlmMessage]| -> usize {
        msgs.iter()
            .map(|m| {
                m.content.chars().count()
                    + m.tool_calls
                        .as_deref()
                        .and_then(|tcs| serde_json::to_string(tcs).ok())
                        .map(|s| s.len())
                        .unwrap_or(0)
                    + m.images
                        .as_ref()
                        .map(|imgs| {
                            imgs.iter()
                                .map(|img| img.data.len() + img.media_type.len())
                                .sum()
                        })
                        .unwrap_or(0)
            })
            .sum()
    };

    // Page backwards through history until we hit a soft budget (1.3× char budget)
    // or run out of rows. Keeps oldest-first ordering.
    let soft_cap_chars = ((context_char_budget as f64) * 1.3).round() as usize;
    let mut offset: usize = 0;
    loop {
        let batch = sqlite::list_messages_page(
            &state.db,
            agent_id,
            conversation_id,
            HISTORY_PAGE_SIZE,
            offset,
        )
        .unwrap_or_default();
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        // list_messages_page returns oldest-first; append to preserve global order.
        for row in batch.into_iter() {
            messages.extend(db_row_to_llm(&row));
        }
        offset = offset.saturating_add(batch_len);
        if total_chars(&messages) >= soft_cap_chars {
            break;
        }
        if batch_len < HISTORY_PAGE_SIZE {
            break; // no more rows
        }
    }

    // Sanitize history: fix orphaned tool_calls, dedup tool_results, drop
    // stray tool_results so Anthropic never sees an invalid sequence.
    if messages.len() > 1 {
        let system_msg = messages.remove(0);
        let sanitized = sanitize_messages(messages);
        messages = std::iter::once(system_msg).chain(sanitized).collect();
    }

    // Strip trailing empty assistant messages left by prior empty LLM responses.
    // These produce no content for any provider and can create invalid turn
    // ordering (e.g. consecutive user turns in Gemini after the empty model
    // turn is skipped).
    while messages.len() > 1 {
        if let Some(last) = messages.last()
            && last.role == "assistant"
            && last.content.is_empty()
            && last.tool_calls.as_ref().is_none_or(|tc| tc.is_empty())
        {
            messages.pop();
            continue;
        }
        break;
    }

    // Stale tool-result summarization: tool results outside the recent window
    // have already been processed by the model. Re-sending them verbatim is
    // wasteful; trim them to a short excerpt to reduce token footprint while
    // preserving enough context for the model to understand what happened.
    if messages.len() > 1 + RECENT_WINDOW {
        let stale_until = messages.len() - RECENT_WINDOW;
        for msg in &mut messages[1..stale_until] {
            if msg.role == "tool" {
                let char_count = msg.content.chars().count();
                if char_count > STALE_TOOL_RESULT_MAX_CHARS {
                    let truncated: String = msg
                        .content
                        .chars()
                        .take(STALE_TOOL_RESULT_MAX_CHARS)
                        .collect();
                    msg.content = format!(
                        "{truncated}\n[…{} chars trimmed]",
                        char_count - STALE_TOOL_RESULT_MAX_CHARS
                    );
                }
            }
        }
    }

    // -- Auto-compaction: summarize old turns into memory when near capacity
    //
    // Trigger: total assembled chars ≥ COMPACT_THRESHOLD (85%) of budget
    //          AND enough messages exist AND cooldown has elapsed.
    //          At COMPACT_EMERGENCY_THRESHOLD (95%) the cooldown is bypassed.
    //
    // On trigger:
    //   1. Extract the oldest dialogue turns (everything except the recent tail).
    //   2. Summarize them via a single LLM call.
    //   3. Write the summary as a short-term memory block (ages normally).
    //   4. **Remove the compacted messages** from the in-flight array.
    //   5. Inject the summary after the system prompt so context is preserved.
    //   6. Update cooldown stamp.
    //
    // On failure: log a warning and fall through to the existing hard-trim loop.
    {
        let current_total = total_chars(&messages);
        let usage_ratio = current_total as f64 / context_char_budget as f64;
        let non_system_count = messages.iter().filter(|m| m.role != "system").count();

        let should_compact =
            usage_ratio >= COMPACT_THRESHOLD && non_system_count >= COMPACT_MIN_MESSAGES;

        if should_compact {
            // Cooldown check — bypassed at emergency threshold.
            let is_emergency = usage_ratio >= COMPACT_EMERGENCY_THRESHOLD;
            let last_compact_turn: i64 = {
                let blocks = sqlite::get_active_blocks(&state.db, agent_id).unwrap_or_default();
                blocks
                    .iter()
                    .find(|(label, ..)| label == "__compact_turn")
                    .and_then(|(_, val, ..)| val.trim().parse::<i64>().ok())
                    .unwrap_or(0)
            };
            let cooldown_ok =
                is_emergency || (current_turn - last_compact_turn) >= COMPACT_COOLDOWN_TURNS;

            if cooldown_ok {
                // Determine the chunk to summarize: messages[1..end-COMPACT_KEEP_RECENT]
                // (skip system prompt at [0], keep recent tail).
                let keep_start = messages.len().saturating_sub(COMPACT_KEEP_RECENT);
                if keep_start > 1 {
                    let chunk = &messages[1..keep_start];
                    let has_dialogue = chunk
                        .iter()
                        .any(|m| m.role == "user" || m.role == "assistant");

                    if has_dialogue {
                        let compacted_count = chunk.len();
                        tracing::info!(
                            "Auto-compaction triggered for agent '{}': usage {:.0}% ({}/{} chars), \
                             summarizing {} messages{}",
                            agent_id,
                            usage_ratio * 100.0,
                            current_total,
                            context_char_budget,
                            compacted_count,
                            if is_emergency {
                                " [EMERGENCY — cooldown bypassed]"
                            } else {
                                ""
                            },
                        );

                        match summarize_for_compaction(state, &agent.model, chunk).await {
                            Ok(summary) => {
                                // Write summary into short-term memory.
                                let label = format!("summary:compact:turn{current_turn}");
                                let desc = Some("Auto-compacted conversation history");
                                let _ = sqlite::upsert_memory_block(
                                    &state.db, agent_id, &label, &summary, desc, None,
                                );
                                // Update cooldown stamp.
                                let _ = sqlite::upsert_memory_block(
                                    &state.db,
                                    agent_id,
                                    "__compact_turn",
                                    &current_turn.to_string(),
                                    Some("Internal: last auto-compaction turn"),
                                    None,
                                );

                                // **Remove the compacted messages** from the array.
                                // This is the critical fix: without removal, the summary
                                // injection INCREASES total size and the hard-trim loop
                                // immediately evicts the summary.
                                messages.drain(1..keep_start);

                                // Inject summary right after the system prompt (index 1).
                                messages.insert(1, LlmMessage {
                                    role: "user".to_string(),
                                    content: format!(
                                        "[System: the following is an auto-compacted summary \
                                         of earlier conversation history ({compacted_count} messages). \
                                         Continue from where the summary leaves off.]\n\n{summary}"
                                    ),
                                    tool_call_id: None,
                                    tool_calls: None,
                                    images: None,
                                });
                                // Add an empty assistant ack so provider turn ordering is valid.
                                messages.insert(2, LlmMessage {
                                    role: "assistant".to_string(),
                                    content: "Understood. I have the compacted context and will continue from there.".to_string(),
                                    tool_call_id: None,
                                    tool_calls: None,
                                    images: None,
                                });

                                tracing::info!(
                                    "Auto-compaction complete for agent '{}': removed {} messages, \
                                     summary={} chars, stored as '{}'",
                                    agent_id,
                                    compacted_count,
                                    summary.chars().count(),
                                    label,
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Auto-compaction summarization failed for agent '{}': {}. \
                                     Falling back to hard trim.",
                                    agent_id,
                                    e,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Hard trim: drop oldest non-system messages until total fits the budget.
    // This always runs (whether or not compaction happened above) to guarantee
    // the final message array respects the model's context window.
    //
    // IMPORTANT: remove messages in atomic units to avoid orphaning tool_results.
    // An assistant message with tool_calls + all its following tool results form
    // an indivisible group.  Removing only part of the group causes the LLM to
    // receive an invalid sequence (orphaned tool_results or tool_calls without
    // results), leading to empty responses and infinite re-prompt loops.
    while total_chars(&messages) > context_char_budget && messages.len() > 3 {
        // messages[0] is always the system prompt — inspect messages[1]
        let role = messages[1].role.as_str();
        let has_tool_calls = role == "assistant"
            && messages[1].tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());

        if has_tool_calls {
            // Remove the assistant(tool_calls) message AND all immediately
            // following tool result messages as one atomic unit.
            messages.remove(1); // assistant with tool_calls
            while messages.len() > 1 && messages[1].role == "tool" {
                messages.remove(1);
            }
        } else {
            messages.remove(1);
        }
    }
    // Repair: trimming may have left a non-user turn as the first content message
    // (e.g. an assistant(tool_calls) whose preceding user turn was trimmed away).
    // All providers require the conversation to begin with a user turn; remove
    // any leading non-user messages until one is found or only 2 messages remain.
    // Apply the same atomic-removal rule for assistant+tool groups.
    while messages.len() > 2 && messages[1].role != "user" {
        let has_tool_calls = messages[1].role == "assistant"
            && messages[1].tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
        if has_tool_calls {
            messages.remove(1);
            while messages.len() > 1 && messages[1].role == "tool" {
                messages.remove(1);
            }
        } else {
            messages.remove(1);
        }
    }

    // Re-sanitize after trimming: a second pass guarantees every provider
    // sees a valid sequence (cheap linear scan).
    if messages.len() > 1 {
        let system_msg = messages.remove(0);
        let sanitized = sanitize_messages(messages);
        messages = std::iter::once(system_msg).chain(sanitized).collect();
    }

    // Tool schemas — use agent-specific tools if wired, else all tools
    let agent_tool_ids = sqlite::get_agent_tool_ids(&state.db, agent_id).unwrap_or_default();
    let all_tools = sqlite::list_tools(&state.db).unwrap_or_default();
    let tool_schemas: Vec<Value> = if agent_tool_ids.is_empty() {
        // Not yet wired → provide all registered tools (backwards-compatible)
        all_tools
            .into_iter()
            .filter_map(|t| t.json_schema)
            .collect()
    } else {
        all_tools
            .into_iter()
            .filter(|t| agent_tool_ids.contains(&t.id))
            .filter_map(|t| t.json_schema)
            .collect()
    };

    // Lazy tool loading: on long conversations only send "extended" tools
    // (desktop_*, search_*) if they were actually used in the recent window.
    // Core tools (bash, file I/O, memory helpers, etc.) are always included.
    // On short conversations every tool is sent so the model can discover
    // what's available before deciding what to call.
    const EXTENDED_TOOL_PREFIXES: &[&str] = &["desktop_", "search_"];
    let tool_schemas: Vec<Value> = if messages.len() > 1 + RECENT_WINDOW {
        // Collect tool names used in the recent window
        let recent_start = messages.len().saturating_sub(RECENT_WINDOW);
        let mut recently_used: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for msg in &messages[recent_start..] {
            if let Some(calls) = &msg.tool_calls {
                for tc in calls {
                    recently_used.insert(tc.name.as_str());
                }
            }
        }
        tool_schemas
            .into_iter()
            .filter(|schema| {
                let name = schema["name"].as_str().unwrap_or("");
                let is_extended = EXTENDED_TOOL_PREFIXES.iter().any(|p| name.starts_with(p));
                !is_extended || recently_used.contains(name)
            })
            .collect()
    } else {
        tool_schemas
    };

    Ok((agent.model, messages, tool_schemas))
}

/// Convert a DB MessageRow to one or more LlmMessages.
fn db_row_to_llm(row: &MessageRow) -> Vec<LlmMessage> {
    match row.role.as_str() {
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

fn new_msg_id() -> String {
    format!("msg-{}", Uuid::new_v4())
}

fn persist(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    role: &str,
    content: Value,
) {
    let row = MessageRow {
        id: new_msg_id(),
        agent_id: agent_id.to_string(),
        conversation_id: conversation_id.map(String::from),
        role: role.to_string(),
        content,
    };
    let _ = sqlite::insert_message(&state.db, &row);
    // Touch the conversation's updated_at so list order stays current
    if let Some(conv_id) = conversation_id {
        let _ = sqlite::touch_conversation(&state.db, conv_id);
    }
}

/// Extract and validate conversation_id from request body.
/// If present and non-empty, verifies it exists in the DB.
/// Returns Ok(Some(id)) | Ok(None) | Err(response).
fn resolve_conversation(
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

pub async fn send_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let conv_id = match resolve_conversation(&state, &agent_id, &body) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let conv_id_ref = conv_id.as_deref();

    if body["role"].as_str() == Some("tool") {
        return handle_tool_return_blocking(&state, &agent_id, conv_id_ref, &body).await;
    }

    let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
    };

    // Collect optional inline images from the request body.
    // Each element must have "media_type" and "data" (base64) fields.
    let req_images: Option<Vec<MessageImage>> = body["images"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .filter(|v: &Vec<_>| !v.is_empty());

    // 1. Persist user message FIRST (skip for ephemeral system injections)
    let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
    if !is_ephemeral {
        let mut user_content = json!({ "content": input });
        if let Some(imgs) = &req_images {
            user_content["images"] = serde_json::to_value(imgs).unwrap_or(Value::Null);
        }
        persist(&state, &agent_id, conv_id_ref, "user", user_content);
    }

    // 2. Build context from DB (includes the message we just persisted)
    let (model, mut messages, tools) =
        match build_context(&state, &agent_id, conv_id_ref, false).await {
            Ok(ctx) => ctx,
            Err(e) => return err(StatusCode::NOT_FOUND, &e),
        };

    // 2b. Ephemeral messages were not persisted — inject into context so the
    // LLM actually sees them.  Without this the re-prompt text is silently lost.
    if is_ephemeral {
        messages.push(LlmMessage {
            role: "user".to_string(),
            content: input.clone(),
            tool_call_id: None,
            tool_calls: None,
            images: req_images.clone(),
        });
    }

    // 3. Call LLM
    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp
                .tool_calls
                .iter()
                .filter_map(|tc| serde_json::to_value(tc).ok())
                .collect();
            // Skip persisting empty assistant responses — they clutter the
            // conversation and can produce invalid turn ordering on next load.
            let has_content = resp.content.as_ref().is_some_and(|s| !s.is_empty());
            let has_tools = !resp.tool_calls.is_empty();
            if has_content || has_tools {
                persist(
                    &state,
                    &agent_id,
                    conv_id_ref,
                    "assistant",
                    json!({
                        "content": resp.content.clone().unwrap_or_default(),
                        "tool_calls": tool_calls_json
                    }),
                );
            }

            let mut out: Vec<Value> = vec![];
            if let Some(text) = &resp.content {
                out.push(json!({ "message_type": "assistant_message", "content": text }));
            }
            for tc in &resp.tool_calls {
                out.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }));
            }
            Json(json!({ "messages": out, "conversation_id": conv_id })).into_response()
        }
        Err(e) => {
            tracing::error!("LLM error: {e}");
            err(StatusCode::BAD_GATEWAY, &e.to_string())
        }
    }
}

async fn handle_tool_return_blocking(
    state: &AppState,
    agent_id: &str,
    conv_id: Option<&str>,
    body: &Value,
) -> Response {
    let tr = &body["tool_return"];
    let call_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
    let content = tr["content"].as_str().unwrap_or("").to_string();

    persist(
        state,
        agent_id,
        conv_id,
        "tool",
        json!({
            "content": content, "tool_call_id": call_id
        }),
    );

    match sqlite::pending_tool_results(&state.db, agent_id, conv_id) {
        Ok((received, expected)) if received < expected => {
            tracing::debug!("Tool results: {received}/{expected} — waiting");
            return Json(json!({ "messages": [], "conversation_id": conv_id })).into_response();
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        _ => {}
    }

    let (model, messages, tools) = match build_context(state, agent_id, conv_id, true).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp
                .tool_calls
                .iter()
                .filter_map(|tc| serde_json::to_value(tc).ok())
                .collect();
            let has_content = resp.content.as_ref().is_some_and(|s| !s.is_empty());
            let has_tools = !resp.tool_calls.is_empty();
            if has_content || has_tools {
                persist(
                    state,
                    agent_id,
                    conv_id,
                    "assistant",
                    json!({
                        "content": resp.content.clone().unwrap_or_default(),
                        "tool_calls": tool_calls_json
                    }),
                );
            }
            let mut out: Vec<Value> = vec![];
            if let Some(text) = &resp.content {
                out.push(json!({ "message_type": "assistant_message", "content": text }));
            }
            for tc in &resp.tool_calls {
                out.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }));
            }
            Json(json!({ "messages": out, "conversation_id": conv_id })).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

// -- POST /v1/agents/:id/messages/stream  (SSE)

pub async fn stream_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let conv_id: Option<String> = match resolve_conversation(&state, &agent_id, &body) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let conv_str = conv_id.clone();
    let conv_id_ref = conv_str.as_deref();

    let is_tool_return = body["role"].as_str() == Some("tool");

    // 1. Persist incoming message FIRST
    if is_tool_return {
        let tr = &body["tool_return"];
        persist(
            &state,
            &agent_id,
            conv_id_ref,
            "tool",
            json!({
                "content": tr["content"].as_str().unwrap_or(""),
                "tool_call_id": tr["tool_call_id"].as_str().unwrap_or("")
            }),
        );
    } else {
        let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
        };
        // ephemeral=true: system-injected re-prompt — send to LLM but don't
        // persist to the DB so it never appears in conversation history.
        let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
        if !is_ephemeral {
            // Auto-title new conversations from the first user message
            if let Some(cid) = conv_id_ref {
                maybe_set_conv_title(&state, cid, &input);
            }
            persist(
                &state,
                &agent_id,
                conv_id_ref,
                "user",
                json!({ "content": input }),
            );
        }
    }

    // 2. If this was a tool return, check if all results for this turn have arrived.
    if is_tool_return {
        match sqlite::pending_tool_results(&state.db, &agent_id, conv_id_ref) {
            Ok((received, expected)) if received < expected => {
                tracing::debug!("Stream: tool results {received}/{expected} — waiting");
                let s = futures::stream::once(async {
                    Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]"))
                });
                return Sse::new(s).into_response();
            }
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            _ => {}
        }
    }

    // 3. Build context from DB
    let (model, mut messages, tools) =
        match build_context(&state, &agent_id, conv_id_ref, is_tool_return).await {
            Ok(ctx) => ctx,
            Err(e) => return err(StatusCode::NOT_FOUND, &e),
        };

    // 3b. Ephemeral messages were not persisted — inject into context so the
    // LLM actually sees them.  Without this the re-prompt text is silently
    // lost and the LLM is called with the same context that already produced
    // an empty response.
    if !is_tool_return {
        let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
        if is_ephemeral && let Some(input) = body["input"].as_str().filter(|s| !s.is_empty()) {
            messages.push(LlmMessage {
                role: "user".to_string(),
                content: input.to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            });
        }
    }

    let background = body["background"].as_bool().unwrap_or(false);
    // Create a run for background (and also for foreground — keeps history for reconnect)
    let run = sqlite::create_run(&state.db, &agent_id, conv_id_ref);
    let run_id: Option<String> = run.ok().map(|r| r.id);

    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();
    let conv_id_clone = conv_str.clone();
    let run_id_clone = run_id.clone();
    let db_clone = state.db.clone();

    // Open LLM stream.
    // On failure, return a well-formed SSE stream with an error event + [DONE]
    // instead of a raw HTTP 502. This prevents reqwest_eventsource from
    // triggering the client's SSE fallback (which would re-persist the user
    // message and call the blocking endpoint — duplicating DB entries).
    let llm_stream = match state.llm.stream(&req).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = e.to_string();
            tracing::error!("LLM stream open failed: {err_msg}");
            if let Some(rid) = &run_id {
                let _ = sqlite::finish_run(&state.db, rid, "failed");
            }
            let s = futures::stream::iter([
                Ok::<Event, std::convert::Infallible>(
                    Event::default().data(json!({ "error": err_msg }).to_string()),
                ),
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]")),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let acc = std::sync::Arc::new(std::sync::Mutex::new((String::new(), Vec::<Value>::new())));
    let acc_clone = acc.clone();
    // Accumulate token usage across chunks
    let usage_acc = std::sync::Arc::new(std::sync::Mutex::new(TokenUsage::default()));
    let usage_acc2 = usage_acc.clone();

    // First SSE event: metadata (conversation_id + run_id)
    let meta_event = {
        let data = json!({
            "message_type": "stream_start",
            "conversation_id": conv_str,
            "run_id": run_id,
        });
        futures::stream::once(async move {
            Ok::<Event, std::convert::Infallible>(Event::default().data(data.to_string()))
        })
    };

    let sse_stream = futures::StreamExt::map(llm_stream, move |chunk: cade_ai::Result<StreamChunk>| {
        // Persist each event to run_events so the stream is resumable
        let emit = |data: Value| -> Event {
            if let Some(rid) = &run_id_clone
                && let Ok(seq) = sqlite::append_run_event(&db_clone, rid, &data.to_string())
            {
                let mut d = data.clone();
                if let Some(obj) = d.as_object_mut() {
                    obj.insert("run_id".to_string(), serde_json::Value::String(rid.clone()));
                    obj.insert("seq_id".to_string(), serde_json::Value::Number(seq.into()));
                }
                return Event::default().data(d.to_string());
            }
            Event::default().data(data.to_string())
        };

        let event = match chunk {
            Ok(StreamChunk::Reasoning(text)) => {
                emit(json!({ "message_type": "reasoning_message", "reasoning": text }))
            }
            Ok(StreamChunk::Text(text)) => {
                if let Ok(mut g) = acc_clone.lock() {
                    g.0.push_str(&text);
                }
                emit(json!({ "message_type": "assistant_message", "content": text }))
            }
            Ok(StreamChunk::ToolCall(tc)) => {
                if let Ok(mut g) = acc_clone.lock()
                    && let Ok(v) = serde_json::to_value(&tc)
                {
                    g.1.push(v);
                }
                emit(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }))
            }
            Ok(StreamChunk::Usage(u)) => {
                if let Ok(mut acc) = usage_acc2.lock() {
                    acc.input_tokens += u.input_tokens;
                    acc.output_tokens += u.output_tokens;
                }
                // Emit usage_statistics event for client-side display
                emit(json!({
                    "message_type":      "usage_statistics",
                    "input_tokens":      u.input_tokens,
                    "output_tokens":     u.output_tokens,
                    "cache_read_tokens":  u.cache_read_tokens,
                    "cache_write_tokens": u.cache_write_tokens,
                    "model":             u.model,
                }))
            }
            Ok(StreamChunk::FinishReason(reason)) => emit(json!({
                "message_type": "finish_reason",
                "reason": reason,
            })),
            Ok(StreamChunk::Done) => {
                if let Ok(g) = acc_clone.lock() {
                    // Skip persisting empty assistant responses — they clutter
                    // the conversation and produce invalid turn ordering on
                    // next context load (e.g. Gemini consecutive-user-turn 400).
                    if !g.0.is_empty() || !g.1.is_empty() {
                        persist(
                            &state_clone,
                            &agent_id_clone,
                            conv_id_clone.as_deref(),
                            "assistant",
                            json!({
                                "content": g.0,
                                "tool_calls": g.1
                            }),
                        );
                    }
                }
                if let Some(rid) = &run_id_clone {
                    let _ = sqlite::finish_run(&db_clone, rid, "completed");
                }
                Event::default().data("[DONE]")
            }
            Err(e) => {
                if let Some(rid) = &run_id_clone {
                    let _ = sqlite::finish_run(&db_clone, rid, "failed");
                }
                Event::default().data(json!({ "error": e.to_string() }).to_string())
            }
        };
        Ok::<Event, std::convert::Infallible>(event)
    });

    drop(acc);
    drop(usage_acc);
    let _ = background;

    Sse::new(futures::StreamExt::chain(meta_event, sse_stream)).into_response()
}

// -- Helpers

/// Set conversation title from first user message if title is still empty.
fn maybe_set_conv_title(state: &AppState, conv_id: &str, text: &str) {
    if let Ok(Some(c)) = sqlite::get_conversation(&state.db, conv_id)
        && c.title.is_empty()
    {
        let title: String = text.chars().take(CONV_TITLE_MAX).collect();
        let title = title.trim().to_string();
        if !title.is_empty() {
            let _ = sqlite::update_conversation_title(&state.db, conv_id, &title);
        }
    }
}

// -- Helpers

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}

use super::*;
use crate::server::state::AppState;
use cade_store::sqlite::{self};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use cade_ai::{LlmMessage, catalogue};
use serde_json::{Value, json};

pub(crate) fn sanitize_messages(messages: Vec<LlmMessage>) -> Vec<LlmMessage> {
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

// -- Context builder
//
// Key design rule:
//   Callers PERSIST a message to SQLite BEFORE calling build_context.
//   build_context loads everything from SQLite — no new_message parameter.
//   This prevents the double-message bug that breaks tool_use/tool_result ordering.

/// Group a flat, oldest-first list of [`LlmMessage`]s into logical turns.
///
/// A turn starts at each `user` message and includes every following non-`user`
/// message (assistant text, tool calls, tool results) up to but not including the
/// next `user` message.
///
/// Turn grouping is the unit of inclusion/exclusion in the budget-based context
/// builder.  A turn is always added or dropped as a whole so that `tool_call` /
/// `tool_result` pairs are never split at the context boundary — a split would
/// produce an invalid message sequence and a provider 400 error.
pub(crate) fn group_into_turns(messages: &[LlmMessage]) -> Vec<Vec<LlmMessage>> {
    let mut turns: Vec<Vec<LlmMessage>> = Vec::new();
    let mut current: Vec<LlmMessage> = Vec::new();
    for msg in messages {
        // A new user message starts a new turn (flush the current one first).
        if msg.role == "user" && !current.is_empty() {
            turns.push(std::mem::take(&mut current));
        }
        current.push(msg.clone());
    }
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

pub(crate) async fn build_context(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    is_tool_return: bool,
) -> core::result::Result<(String, Vec<LlmMessage>, Vec<Value>), String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    let system_core = assemble_system_prompt_memory(state, &agent, agent_id, is_tool_return);

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


    let context_char_budget = calculate_context_budget(state, agent_id, &agent.model);

    // ── Budget-based, turn-aware history assembly ──────────────────────────
    //
    // Algorithm:
    //  1. Fetch up to MAX_ROWS_SAFETY_CAP rows (safety guard; normal sessions
    //     stay well below this because budget exhausts first).
    //  2. Convert rows to LlmMessages and group them into logical turns so that
    //     tool_call / tool_result pairs are never split at the context boundary.
    //  3. Walk turns from newest to oldest, adding each complete turn while the
    //     char budget allows.  The most-recent turn is ALWAYS included — it
    //     carries the current user request the model must respond to.
    //  4. Reverse back to oldest-first and flatten into the message list.
    let all_rows = sqlite::get_context_window(
        &state.db,
        agent_id,
        conversation_id,
        context_char_budget,
    )
    .unwrap_or_default();

    // Convert DB rows to LlmMessages (oldest-first).
    let all_llm_msgs: Vec<LlmMessage> = all_rows.iter().flat_map(db_row_to_llm).collect();

    // Group into logical turns.
    let mut turns = group_into_turns(&all_llm_msgs);

    // If the window cut off mid-turn, the oldest turn might not start with a user message.
    // Drop it to ensure we never split tool_call/tool_result pairs.
    if let Some(first_msg) = turns.first().and_then(|t| t.first())
        && first_msg.role != "user"
    {
        turns.remove(0);
    }

    // Deduct the already-assembled system-prompt size from the message budget.
    let system_chars = messages
        .first()
        .map(|m| m.content.chars().count())
        .unwrap_or(0);
    let message_budget = context_char_budget.saturating_sub(system_chars);

    let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
    let mut budget_used: usize = 0;
    let mut omitted_turns: usize = 0;

    let turns_len = turns.len();

    for mut turn in turns.into_iter().rev() {
        // Approximate turn cost: sum of content chars + serialised tool-call
        // argument strings (arguments are JSON text and can be large).
        let raw_chars: usize = turn
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

        let mut turn_chars = raw_chars;

        if selected.is_empty() {
            // Always include the most-recent turn regardless of size.
            selected.push(turn);
            budget_used += turn_chars;
        } else if budget_used + turn_chars <= message_budget {
            selected.push(turn);
            budget_used += turn_chars;
        } else {
            // Attempt Tool Result Truncation before dropping the turn
            let deficit = (budget_used + turn_chars).saturating_sub(message_budget);
            let tool_results_chars: usize = turn
                .iter()
                .filter(|m| m.role == "tool")
                .map(|m| m.content.chars().count())
                .sum();
            
            let margin = 200; // minimum characters remaining plus truncation text length
            if tool_results_chars > deficit + margin {
                let to_cut = deficit + margin;
                let mut cut_remaining = to_cut;
                
                for m in turn.iter_mut().filter(|m| m.role == "tool") {
                    let len = m.content.chars().count();
                    if len > margin && cut_remaining > 0 {
                        let cut_here = cut_remaining.min(len.saturating_sub(margin));
                        let keep = len - cut_here;
                        let mut new_content: String = m.content.chars().take(keep).collect();
                        new_content.push_str(&format!(
                            "\n... [{} chars truncated to fit context window]",
                            cut_here
                        ));
                        m.content = new_content;
                        cut_remaining -= cut_here;
                    }
                    if cut_remaining == 0 {
                        break;
                    }
                }
                
                if cut_remaining == 0 {
                    turn_chars -= to_cut;
                    if budget_used + turn_chars <= message_budget {
                        selected.push(turn);
                        budget_used += turn_chars;
                        continue;
                    }
                }
            }
            
            omitted_turns += 1;
        }
    }

    // ── Proactive overflow signal ──────────────────────────────────────────
    // Trigger consolidation early when context usage ≥ 80%, even if no turns
    // were dropped yet.  This gives the Sleeptime task time to produce a
    // summary before the next request actually overflows.
    const PROACTIVE_CONSOLIDATION_THRESHOLD: f64 = 0.80;
    let usage_fraction = if message_budget > 0 {
        budget_used as f64 / message_budget as f64
    } else {
        0.0
    };
    let needs_proactive = usage_fraction >= PROACTIVE_CONSOLIDATION_THRESHOLD;

    if omitted_turns > 0 || needs_proactive {
        if omitted_turns > 0 {
            tracing::debug!(
                "build_context [{}]: {}/{} turns fit in budget \
                 ({} chars used / {} budget); {} older turn(s) omitted — \
                 agent can recover them via conversation_search / search_memory",
                agent_id,
                selected.len(),
                turns_len,
                budget_used,
                message_budget,
                omitted_turns,
            );
        } else {
            tracing::debug!(
                "build_context [{}]: proactive consolidation signal at {:.0}% usage \
                 ({} chars / {} budget)",
                agent_id,
                usage_fraction * 100.0,
                budget_used,
                message_budget,
            );
        }
        // Signal the Sleeptime consolidation task.  After 60 s of inactivity
        // it will summarise the dropped turns into the `session_summary` block.
        let mut activity = state.agent_activity.write().await;
        let entry =
            activity
                .entry(agent_id.to_string())
                .or_insert(crate::server::state::AgentActivity {
                    last_active_ts: chrono::Utc::now().timestamp(),
                    needs_consolidation: true,
                    conversation_id: conversation_id.map(String::from),
                });
        entry.needs_consolidation = true;
        if conversation_id.is_some() {
            entry.conversation_id = conversation_id.map(String::from);
        }

        // ── Surgical tool-output pruning ───────────────────────────────────
        // When turns are being dropped, compact old tool outputs in the DB so
        // future requests can fit more turns.  Keeps the last PRUNE_PROTECT_CHARS
        // of tool output at full fidelity; older large outputs are replaced with
        // a "[tool output compacted — N chars]" placeholder.
        if omitted_turns > 0 {
            const PRUNE_PROTECT_CHARS: usize = 120_000; // ~40k tokens × 3 chars/token
            const PRUNE_MIN_CHARS: usize = 200;         // only compact outputs > 200 chars
            match sqlite::compact_old_tool_outputs(
                &state.db,
                agent_id,
                conversation_id,
                PRUNE_PROTECT_CHARS,
                PRUNE_MIN_CHARS,
            ) {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        "build_context [{}]: pruned {} old tool outputs",
                        agent_id, n,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "build_context [{}]: tool-output pruning failed: {}",
                        agent_id, e,
                    );
                }
                _ => {}
            }
        }
    }

    // Reverse (was newest-first) back to oldest-first, then flatten.
    selected.reverse();
    messages.extend(selected.into_iter().flatten());

    // Sanitize history: fix orphaned tool_calls, dedup tool_results, drop
    // stray tool_results so Anthropic never sees an invalid sequence.
    if messages.len() > 1 {
        let system_msg = messages.remove(0);
        let sanitized = sanitize_messages(messages);
        messages = std::iter::once(system_msg).chain(sanitized).collect();
    }

    // Strip trailing empty assistant messages left by prior empty LLM responses.
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

    // Ensure the conversation begins with a user turn (all providers require this).
    while messages.len() > 2 && messages[1].role != "user" {
        let has_tool_calls = messages[1].role == "assistant"
            && messages[1]
                .tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty());
        if has_tool_calls {
            messages.remove(1);
            while messages.len() > 1 && messages[1].role == "tool" {
                messages.remove(1);
            }
        } else {
            messages.remove(1);
        }
    }

    // Re-sanitize after trimming
    if messages.len() > 1 {
        let system_msg = messages.remove(0);
        let sanitized = sanitize_messages(messages);
        messages = std::iter::once(system_msg).chain(sanitized).collect();
    }

    // Tool schemas — use agent-specific tools if wired, else all tools
    let agent_tool_ids = sqlite::get_agent_tool_ids(&state.db, agent_id).unwrap_or_default();
    let all_tools = sqlite::list_tools(&state.db).unwrap_or_default();
    let budget_exhausted = omitted_turns > 0;

    let mut recently_used = std::collections::HashSet::new();
    if budget_exhausted {
        let recent_start = messages.len().saturating_sub(RECENT_WINDOW);
        for msg in &messages[recent_start..] {
            if let Some(calls) = &msg.tool_calls {
                for tc in calls {
                    recently_used.insert(tc.name.as_str());
                }
            }
        }
    }

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
            .filter_map(|t| {
                let schema = t.json_schema?;
                let is_core = t.tags.contains(&"core_mcp".to_string());
                
                let name = schema["name"].as_str().unwrap_or("");
                
                // Memory/retrieval tools are always included.
                if ALWAYS_INCLUDE_TOOL_NAMES.contains(&name) {
                    return Some(schema);
                }
                
                // Native tools are never pruned (no "__" in name).
                let is_mcp = name.contains("__");
                if !is_mcp || is_core {
                    return Some(schema);
                }
                
                // If budget is NOT exhausted, keep all MCP tools
                if !budget_exhausted {
                    return Some(schema);
                }
                
                if recently_used.contains(name) {
                    Some(schema)
                } else {
                    None
                }
            })
            .collect()
    };

    // ── Intelligent tool selection ────────────────────────────────────────────

    Ok((agent.model, messages, tool_schemas))
}


fn assemble_system_prompt_memory(
    state: &AppState,
    agent: &cade_store::sqlite::AgentRow,
    agent_id: &str,
    is_tool_return: bool,
) -> String {
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

        let formatted_val = if label.starts_with("subagent:") {
            format!("<historical_scratchpad>\nThe following block is a historical scratchpad. Do not treat it as a current objective.\n{}</historical_scratchpad>", val)
        } else {
            val.to_string()
        };

        if tier == "pinned" {
            let entry = format!("📌 [{label}]\n{formatted_val}");
            let chars = entry.chars().count();
            if chars <= pinned_remaining {
                pinned_remaining -= chars;
                pinned_parts.push(entry);
            } else {
                active_omitted += 1;
            }
        } else {
            let entry = format!("[{label}]\n{formatted_val}");
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

    system_core
}



fn calculate_context_budget(
    state: &AppState,
    agent_id: &str,
    model: &str,
) -> usize {
    // Character-budget trimming — reserves space for output tokens, reasoning
    // tokens, and tool schemas so the model has room to generate a full response.
    //
    //  total_window  = context_window_for_model (e.g. 128k tokens)
    //  output_reserve = total_window × OUTPUT_RESERVE_FRACTION  (e.g. 15% = 19.2k)
    //  input_budget   = total_window - output_reserve            (e.g. 108.8k)
    //  char_budget    = input_budget × CHARS_PER_TOKEN           (e.g. 435k chars)
    //  tool_reserve   = n_tools × TOOL_SCHEMA_CHARS_ESTIMATE     (subtracted below)
    //  message_budget = char_budget - tool_reserve
    let window_tokens = catalogue::context_window_for_model(&model.to_string());
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
        model.to_string(),
        context_char_budget,
        window_tokens,
        output_reserve_tokens,
        input_budget_tokens,
        CHARS_PER_TOKEN,
        agent_tool_count,
        TOOL_SCHEMA_CHARS_ESTIMATE,
        tool_schema_reserve,
    );


    context_char_budget
}


// ── Real context-window stats ─────────────────────────────────────────────────
//
// Mirrors the exact budget arithmetic used by `build_context` so the CLI can
// show accurate turn counts and char usage without guessing from token percentages.

/// GET /v1/agents/:id/context?conversation_id=<id>
///
/// Returns accurate server-side context-window accounting: how many turns are
/// included vs omitted, chars used vs budget, and whether a Sleeptime
/// consolidation is pending.
pub async fn get_context_stats_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let conv_id = params.get("conversation_id").map(String::as_str);
    match compute_context_stats(&state, &agent_id, conv_id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, &e).into_response(),
    }
}

/// Compute context-window stats without assembling the full message list for the LLM.
/// Shares all budget constants with `build_context` so the numbers are identical.
pub(crate) async fn compute_context_stats(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> core::result::Result<Value, String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    // ── Same budget formula as build_context ────────────────────────────────
    let window_tokens = catalogue::context_window_for_model(&agent.model);
    let output_reserve = ((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize;
    let input_budget_tokens = (window_tokens as usize).saturating_sub(output_reserve);
    let agent_tool_count = sqlite::get_agent_tool_ids(&state.db, agent_id)
        .unwrap_or_default()
        .len()
        .max(1);
    let tool_schema_reserve = agent_tool_count * TOOL_SCHEMA_CHARS_ESTIMATE;
    let context_char_budget = {
        let raw = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
        raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS)
            .saturating_sub(tool_schema_reserve)
            .max(MIN_CONTEXT_CHARS)
    };

    // ── Load and group messages (same as build_context) ─────────────────────
    let all_rows =
        sqlite::get_context_window(&state.db, agent_id, conversation_id, context_char_budget)
            .unwrap_or_default();

    let all_llm_msgs: Vec<LlmMessage> = all_rows.iter().flat_map(db_row_to_llm).collect();

    let mut turns = group_into_turns(&all_llm_msgs);
    if let Some(first_msg) = turns.first().and_then(|t| t.first())
        && first_msg.role != "user"
    {
        turns.remove(0);
    }
    let total_turns = turns.len();

    // System prompt and memory chars (overhead subtracted from the message budget)
    let system_chars = agent
        .system_prompt
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let memory_chars: usize = sqlite::get_active_blocks(&state.db, agent_id)
        .unwrap_or_default()
        .iter()
        .map(|(_, v, _, _, _)| v.chars().count())
        .sum();
    let message_budget = context_char_budget.saturating_sub(system_chars);

    // ── Turn selection (same walk as build_context) ──────────────────────────
    let mut turns_included = 0usize;
    let mut turns_omitted = 0usize;
    let mut chars_used = 0usize;

    for turn in turns.iter().rev() {
        let turn_chars: usize = turn
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

        if turns_included == 0 || chars_used + turn_chars <= message_budget {
            turns_included += 1;
            chars_used += turn_chars;
        } else {
            turns_omitted += 1;
        }
    }

    // ── Consolidation flag ───────────────────────────────────────────────────
    let needs_consolidation = {
        let activity = state.agent_activity.read().await;
        activity
            .get(agent_id)
            .map(|a| a.needs_consolidation)
            .unwrap_or(false)
    };

    Ok(json!({
        "model":                   agent.model,
        "window_tokens":           window_tokens,
        "turns_total":             total_turns,
        "turns_included":          turns_included,
        "turns_omitted":           turns_omitted,
        "chars_used":              chars_used,
        "message_budget_chars":    message_budget,
        "memory_chars":            memory_chars,
        "system_prompt_chars":     system_chars,
        "tool_count":              agent_tool_count,
        "tool_schema_reserve_chars": tool_schema_reserve,
        "needs_consolidation":     needs_consolidation,
    }))
}

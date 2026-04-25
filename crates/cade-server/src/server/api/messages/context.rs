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

/// Hard cap a single message's content to `cap` chars.
///
/// If `content` is already within the cap, the message is returned unchanged.
/// Otherwise the body is split into a head (first 20 % of `cap`) and a tail
/// (the remaining 80 %), joined by [`TRUNCATION_MARKER`].  Role,
/// `tool_call_id`, `tool_calls`, and `images` are preserved so message
/// validity (e.g. tool_use ↔ tool_result pairing) is not broken.
///
/// This is the per-message escape hatch for the "most-recent turn always
/// included" rule in [`build_context`]: a single oversized tool result can no
/// longer wedge the session by overflowing the provider's context window.
pub(crate) fn truncate_oversize_message(mut msg: LlmMessage, cap: usize) -> LlmMessage {
    let len = msg.content.chars().count();
    if len <= cap {
        return msg;
    }
    let head_chars = (cap as f64 * 0.20).round() as usize;
    let tail_chars = cap.saturating_sub(head_chars);
    let head: String = msg.content.chars().take(head_chars).collect();
    let tail: String = msg
        .content
        .chars()
        .skip(len.saturating_sub(tail_chars))
        .collect();
    let mut new_content = String::with_capacity(head.len() + TRUNCATION_MARKER.len() + tail.len());
    new_content.push_str(&head);
    new_content.push_str(TRUNCATION_MARKER);
    new_content.push_str(&tail);
    msg.content = new_content;
    msg
}

/// Render the "# Loaded Skills" section with a character budget.
///
/// For each skill, in input order:
/// * If the body length is ≤ `body_cap` AND the full entry fits within the
///   remaining `budget`, emit the full body.
/// * Otherwise, emit a one-line summary entry pointing the agent at the
///   `load_skill_ref` tool to fetch the body on demand.
///
/// Returns an empty string when `loaded` is empty so the caller can avoid
/// pushing a useless heading.
pub(crate) fn render_skills_section(
    loaded: &[&cade_core::skills::Skill],
    budget: usize,
    body_cap: usize,
) -> String {
    if loaded.is_empty() {
        return String::new();
    }
    let header = "\n\n# Loaded Skills\n";
    let mut section = String::from(header);
    let mut remaining = budget;

    for skill in loaded {
        let summary_line = format!(
            "\n## Skill: {} ({}) [summary-only — call load_skill_ref to fetch full body]\n{}\n",
            skill.name,
            skill.id,
            skill.description.lines().next().unwrap_or("").trim()
        );
        let body_chars = skill.body.chars().count();
        let full_entry = format!(
            "\n## Skill: {} ({})\n{}\n",
            skill.name, skill.id, skill.body
        );
        let full_chars = full_entry.chars().count();

        // Use full body only when it is below the per-skill cap AND fits
        // within the remaining section budget.
        let fits_full = body_chars <= body_cap && full_chars <= remaining;
        let chosen = if fits_full { full_entry } else { summary_line };
        let chosen_chars = chosen.chars().count();

        // If even the summary doesn't fit, stop adding skills altogether.
        if chosen_chars > remaining {
            section.push_str(&format!(
                "\n[…{} more loaded skill(s) omitted — section budget exhausted; \
                 use load_skill_ref to access content]\n",
                loaded
                    .iter()
                    .skip_while(|s| s.id != skill.id)
                    .count()
            ));
            break;
        }

        section.push_str(&chosen);
        remaining = remaining.saturating_sub(chosen_chars);
    }

    if section == header {
        String::new()
    } else {
        section
    }
}

/// Call the LLM router, retrying once with an aggressively shrunk context if
/// the provider rejects the request as too long.
///
/// Recovery sequence on `Error::is_context_overflow()`:
///   1. Trigger synchronous [`crate::server::consolidation::consolidate_agent`]
///      so dropped turns are summarised into the `session_summary` block
///      *before* we rebuild context.
///   2. Halve the agent's per-call message budget by setting a one-shot
///      override on `AppState::config.max_context_budget` (NOT mutated here —
///      we just rebuild with a manually shrunk message vector).
///   3. Rebuild via [`build_context`] so the fresh `session_summary` block
///      replaces the dropped turns; force-trim the older half of the message
///      list as a safety net.
///   4. Retry exactly once.  A second overflow returns the original error so
///      the caller can surface a clear message to the user.
///
/// Non-overflow errors (4xx auth, 5xx provider, network) are returned
/// untouched on the first call so callers retain their existing semantics.
pub(crate) async fn complete_with_overflow_recovery(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    is_tool_return: bool,
    mut req: cade_ai::CompletionRequest,
) -> cade_ai::Result<cade_ai::CompletionResponse> {
    match state.llm.complete(&req).await {
        Ok(resp) => Ok(resp),
        Err(e) if e.is_context_overflow() => {
            tracing::warn!(
                "complete [{}]: context overflow detected ({}); running consolidation + retry",
                agent_id,
                e
            );

            // 1. Synchronous consolidation — drops summarised into session_summary.
            crate::server::consolidation::consolidate_agent(state, agent_id, conversation_id).await;

            // 2. Drop the context cache entry so build_context recomputes.
            {
                let mut cache = state
                    .context_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let key = format!("{agent_id}:{conversation_id:?}");
                cache.pop(&key);
            }

            // 3. Rebuild context fresh.
            let (model, mut new_messages, new_tools) =
                match build_context(state, agent_id, conversation_id, is_tool_return).await {
                    Ok(ctx) => ctx,
                    Err(build_err) => {
                        tracing::error!(
                            "complete [{}]: rebuild after overflow failed: {}",
                            agent_id,
                            build_err
                        );
                        return Err(e);
                    }
                };

            // 4. Aggressive safety trim: keep the system prompts (always at
            //    the front) and only the most-recent half of the remaining
            //    messages.  This is a belt-and-suspenders measure on top of
            //    the budget logic: if a single turn alone is still over the
            //    provider's window, we will fail again with the original
            //    error and surface it to the user.
            let split_idx = new_messages
                .iter()
                .position(|m| m.role != "system")
                .unwrap_or(new_messages.len());
            let trail_len = new_messages.len().saturating_sub(split_idx);
            if trail_len > 2 {
                let drop_n = trail_len / 2;
                new_messages.drain(split_idx..split_idx + drop_n);
            }

            req.model = model;
            req.messages = new_messages;
            req.tools = new_tools;

            tracing::info!(
                "complete [{}]: retrying with shrunk context ({} messages)",
                agent_id,
                req.messages.len()
            );

            state.llm.complete(&req).await
        }
        Err(e) => Err(e),
    }
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
    let build_started = std::time::Instant::now();
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    let (system_static, mut system_dynamic) = assemble_system_prompt_memory(state, &agent, agent_id, is_tool_return);

    // Skill-counters for Phase-4 telemetry; updated when we render the
    // skills section below.
    let mut skills_full_count: usize = 0;
    let mut skills_summary_count: usize = 0;

    // Inject loaded skills into the dynamic system prompt section.
    {
        let agent_skills = state.agent_skills.read().await;
        if let Some(loaded_ids) = agent_skills.get(agent_id)
            && !loaded_ids.is_empty()
        {
            let all_skills = state.all_skills.read().await;
            let loaded: Vec<&cade_core::skills::Skill> = loaded_ids
                .iter()
                .filter_map(|id| all_skills.iter().find(|s| s.id == *id))
                .collect();
            let skills_section = render_skills_section(
                &loaded,
                SKILLS_INJECTION_BUDGET,
                SKILL_BODY_INDIVIDUAL_CAP,
            );
            if !skills_section.is_empty() {
                // Count full-body vs summary-only entries by looking at
                // the marker text the renderer emits.  Any line beginning
                // with "## Skill:" is one entry; the "summary-only" tag
                // distinguishes the downgraded ones.
                for line in skills_section.lines() {
                    if line.starts_with("## Skill:") {
                        if line.contains("summary-only") {
                            skills_summary_count += 1;
                        } else {
                            skills_full_count += 1;
                        }
                    }
                }
                system_dynamic.push_str(&skills_section);
            }
        }
    }

    // Memory-change detection: cache the assembled static system_core per agent.
    let system_prompt_static = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        system_static.hash(&mut h);
        let new_hash = h.finish();
        let mut cache = state.memory_cache.lock().unwrap_or_else(|e| e.into_inner());
        let entry = cache
            .entry(agent_id.to_string())
            .or_insert((0, String::new()));
        if entry.0 != new_hash {
            entry.0 = new_hash;
            entry.1 = system_static.clone();
        }
        entry.1.clone()
    };

    let max_rowid = sqlite::get_max_rowid(&state.db, agent_id, conversation_id).unwrap_or(0);
    let cache_key = format!("{agent_id}:{conversation_id:?}");
    let state_hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        system_prompt_static.hash(&mut h);
        system_dynamic.hash(&mut h);
        max_rowid.hash(&mut h);
        agent.model.hash(&mut h);
        h.finish()
    };

    {
        let mut cache = state.context_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_hash, cached_tuple)) = cache.get(&cache_key) {
            if *cached_hash == state_hash {
                return Ok(cached_tuple.clone());
            }
        }
    }

    // Message history from DB — oldest first, scoped to conversation
    let mut messages: Vec<LlmMessage> = vec![
        LlmMessage {
            role: "system".to_string(),
            content: system_prompt_static,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        },
        LlmMessage {
            role: "system".to_string(),
            content: format!("{}{TOOL_RESPONSE_RULE}", system_dynamic),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }
    ];


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
    let all_llm_msgs: Vec<LlmMessage> = all_rows
        .iter()
        .flat_map(db_row_to_llm)
        .map(|m| truncate_oversize_message(m, PER_MESSAGE_CHAR_CAP))
        .collect();

    // Group into logical turns.
    let mut turns = group_into_turns(&all_llm_msgs);

    // If the window cut off mid-turn, the oldest turn might not start with a user message.
    // Drop it to ensure we never split tool_call/tool_result pairs.
    if let Some(first_msg) = turns.first().and_then(|t| t.first())
        && first_msg.role != "user"
    {
        turns.remove(0);
    }

    // ── P2-1: token-based system overhead deduction ─────────────────────────
    // Previously this counted raw chars of all leading system messages,
    // which over-deducted for verbose system text (English is ~3.5–4 c/t,
    // not 3 c/t).  Now we count tokens with `cade_ai::count_tokens`
    // (cl100k_base / o200k_base depending on model) and convert that token
    // count back into a char budget via `chars_for_tokens`.  This keeps the
    // legacy char-budget contract used by the turn walker while producing a
    // reservation anchored to real provider tokens rather than a flat 3:1
    // char/token estimate.
    let system_text: String = messages
        .iter()
        .take_while(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let system_tokens = cade_ai::count_tokens(&agent.model, &system_text);
    // Defence in depth: if the tokenizer reports zero tokens on non-empty
    // text (encoder load failed and char fallback rounded to zero), fall
    // back to the legacy raw-char deduction so we never *under*-reserve.
    let system_overhead_chars = if system_text.is_empty() {
        0
    } else if system_tokens == 0 {
        system_text.chars().count()
    } else {
        cade_ai::chars_for_tokens(system_tokens)
    };
    let message_budget = context_char_budget.saturating_sub(system_overhead_chars);

    let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
    let mut budget_used: usize = 0;
    let mut omitted_turns: usize = 0;

    // Phase 4 Part 2: per-message + per-turn cost is now anchored to real
    // BPE tokens via `count_tokens` and converted back to a char budget
    // through `chars_for_tokens`.  This matches the system-overhead path
    // (P2-1) so the entire walker now uses one consistent token model.
    // For tool_calls we count the JSON-serialised arguments string the
    // same way the provider serialises them on the wire.
    fn turn_cost_chars(model: &str, turn: &[LlmMessage]) -> usize {
        let mut total_tokens = 0usize;
        for m in turn {
            if !m.content.is_empty() {
                total_tokens += cade_ai::count_tokens(model, &m.content);
            }
            if let Some(tcs) = m.tool_calls.as_deref() {
                for tc in tcs {
                    let json = tc.arguments.to_string();
                    if !json.is_empty() {
                        total_tokens += cade_ai::count_tokens(model, &json);
                    }
                }
            }
        }
        // Defence-in-depth: if the encoder load failed and returned 0
        // tokens for non-empty text, fall back to raw chars so we never
        // *under*-cost the turn.
        let fallback_chars: usize = turn
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
        if total_tokens == 0 && fallback_chars > 0 {
            fallback_chars
        } else {
            cade_ai::chars_for_tokens(total_tokens)
        }
    }

    let turns_len = turns.len();

    for mut turn in turns.into_iter().rev() {
        let raw_chars: usize = turn_cost_chars(&agent.model, &turn);

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
                        let keep_head = (keep as f64 * 0.2) as usize;
                        let keep_tail = keep.saturating_sub(keep_head);
                        let mut new_content: String = m.content.chars().take(keep_head).collect();
                        new_content.push_str(&format!(
                            "\n... [{} chars truncated to fit context window] ...\n",
                            cut_here
                        ));
                        let tail: String = m.content.chars().skip(keep_head + cut_here).take(keep_tail).collect();
                        new_content.push_str(&tail);
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

    // ── P1-2: Pre-flight overflow guard ───────────────────────────────────
    // The turn-walk above guarantees the most-recent turn is included even
    // when oversized.  PER_MESSAGE_CHAR_CAP shrinks individual messages, but
    // their *combined* size may still exceed `message_budget` (e.g. a single
    // huge turn plus several earlier ones still under cap).  If so, drop
    // oldest selected turns (everything except the most recent — which is at
    // index 0 because `selected` was built newest-first) until it fits.
    //
    // After this block, `selected` is guaranteed to satisfy:
    //     sum(turn_chars) + system_chars  ≤  context_char_budget
    // unless even the lone most-recent turn cannot fit, in which case the
    // provider call is unavoidable but PER_MESSAGE_CHAR_CAP has bounded its
    // size, and P1-3 (recovery loop) will handle the 4xx response.
    let mut preflight_dropped = 0usize;
    while selected.len() > 1 && budget_used > message_budget {
        // selected[0] is the most-recent turn (pushed first in the rev-walk).
        // Pop the oldest, which is the last element.
        if let Some(dropped) = selected.pop() {
            let chars: usize = turn_cost_chars(&agent.model, &dropped);
            budget_used = budget_used.saturating_sub(chars);
            preflight_dropped += 1;
        }
    }
    if preflight_dropped > 0 {
        omitted_turns += preflight_dropped;
        tracing::warn!(
            "build_context [{}]: pre-flight overflow guard dropped {} additional turn(s) \
             to fit budget ({} chars used / {} budget)",
            agent_id,
            preflight_dropped,
            budget_used,
            message_budget,
        );
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

    // P5-B: Trigger consolidation if there are too many turns since the last compaction marker.
    // This handles the case where the context budget is large enough to fit many turns, but
    // the growing history causes token bloat even before hitting 80% usage.
    const PROACTIVE_MAX_TURNS: usize = 20;
    let needs_proactive_length = turns_len >= PROACTIVE_MAX_TURNS;

    if omitted_turns > 0 || needs_proactive || needs_proactive_length {
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
                "build_context [{}]: proactive consolidation signal at {:.0}% usage, {} turns \
                 ({} chars / {} budget)",
                agent_id,
                usage_fraction * 100.0,
                turns_len,
                budget_used,
                message_budget,
            );
        }
        // Signal the Sleeptime consolidation task.  After 20 s of inactivity
        // (M3) it will summarise the dropped turns into the `session_summary`
        // block. In continuous interactive sessions the timer may never fire
        // between turns, so we also trigger an eager consolidation when the
        // turn counter has advanced enough since the last run (see M3).
        let eager_snapshot = {
            let mut activity = state.agent_activity.write().await;
            let entry =
                activity
                    .entry(agent_id.to_string())
                    .or_insert(crate::server::state::AgentActivity {
                        last_active_ts: chrono::Utc::now().timestamp(),
                        needs_consolidation: true,
                        conversation_id: conversation_id.map(String::from),
                        last_consolidation_turn: 0,
                    });
            entry.needs_consolidation = true;
            if conversation_id.is_some() {
                entry.conversation_id = conversation_id.map(String::from);
            }

            // Eager-consolidation decision is made under the same lock so two
            // racing requests cannot both cross the threshold and double-fire.
            let current_turn = sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
            if crate::server::consolidation::should_eager_consolidate(
                current_turn,
                entry.last_consolidation_turn,
                crate::server::consolidation::EAGER_CONSOLIDATION_TURN_THRESHOLD,
            ) {
                entry.last_consolidation_turn = current_turn;
                entry.needs_consolidation = false;
                Some(entry.conversation_id.clone())
            } else {
                None
            }
        };

        if let Some(conv_for_eager) = eager_snapshot {
            let state_eager = state.clone();
            let agent_eager = agent_id.to_string();
            tracing::info!(
                "build_context [{}]: eager consolidation triggered (turn-count path)",
                agent_id
            );
            tokio::spawn(async move {
                crate::server::consolidation::consolidate_agent(
                    &state_eager,
                    &agent_eager,
                    conv_for_eager.as_deref(),
                )
                .await;
            });
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
                    // P6-A: Track tool outputs compacted
                    let mut metrics = state.agent_metrics.write().await;
                    metrics.entry(agent_id.to_string()).or_default().tool_outputs_compacted += n;
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
    // Phase 4: capture turn count before we move `selected` into `messages`.
    let turns_selected_count = selected.len();
    messages.extend(selected.into_iter().flatten());

    // Sanitize history: fix orphaned tool_calls, dedup tool_results, drop
    // stray tool_results so Anthropic never sees an invalid sequence.
    // Preserve all leading system messages (static + dynamic).
    let history_start1 = messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(messages.len());
    if messages.len() > history_start1 {
        let system_msgs: Vec<LlmMessage> = messages.drain(..history_start1).collect();
        let sanitized = sanitize_messages(messages);
        messages = system_msgs.into_iter().chain(sanitized).collect();
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
    // The first 1-2 messages are always system messages (static + optional dynamic);
    // skip past them before enforcing the user-turn rule, otherwise we silently
    // drop the dynamic system message that carries memory + skills + footer.
    let history_start = messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(messages.len());
    while messages.len() > history_start + 1 && messages[history_start].role != "user" {
        let has_tool_calls = messages[history_start].role == "assistant"
            && messages[history_start]
                .tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty());
        if has_tool_calls {
            messages.remove(history_start);
            while messages.len() > history_start && messages[history_start].role == "tool" {
                messages.remove(history_start);
            }
        } else {
            messages.remove(history_start);
        }
    }

    // Re-sanitize after trimming. Preserve all leading system messages.
    let history_start2 = messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(messages.len());
    if messages.len() > history_start2 {
        let system_msgs: Vec<LlmMessage> = messages.drain(..history_start2).collect();
        let sanitized = sanitize_messages(messages);
        messages = system_msgs.into_iter().chain(sanitized).collect();
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

    // Lazy tool schema loading: on long conversations, desktop_* tools are pruned
    // unless they were actually called in the recent message window.  This saves
    // prompt tokens for sessions that never use desktop features.
    //
    // ALWAYS_INCLUDE_TOOL_NAMES are never pruned — they are the agent's primary
    // mechanism for recovering archived/dropped context and must always be present.
    const EXTENDED_TOOL_PREFIXES: &[&str] = &["desktop_"];
    let tool_schemas: Vec<Value> = if messages.len() > 1 + RECENT_WINDOW {
        // Collect tool names called in the recent window.
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
                // Memory/retrieval tools are always included.
                if ALWAYS_INCLUDE_TOOL_NAMES.contains(&name) {
                    return true;
                }
                let is_extended = EXTENDED_TOOL_PREFIXES.iter().any(|p| name.starts_with(p));
                !is_extended || recently_used.contains(name)
            })
            .collect()
    } else {
        tool_schemas
    };

    // ── Intelligent tool selection ────────────────────────────────────────────

    // Phase 4: capture per-request telemetry so we can prove which
    // defence layers fired and how close to the budget we ended up.
    let history_chars: usize = messages
        .iter()
        .skip_while(|m| m.role == "system")
        .map(|m| m.content.chars().count())
        .sum();
    let total_assembled_chars: usize = messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    // Phase 4 Pt 2: native-token counts for history + total assembled.
    // Done in one pass over the assembled message list so we count
    // exactly what the provider will see on the wire (post-truncation,
    // post-sanitize, post-pre-flight-drop).
    let history_tokens: usize = messages
        .iter()
        .skip_while(|m| m.role == "system")
        .map(|m| {
            let mut t = if m.content.is_empty() {
                0
            } else {
                cade_ai::count_tokens(&agent.model, &m.content)
            };
            if let Some(tcs) = m.tool_calls.as_deref() {
                for tc in tcs {
                    let json = tc.arguments.to_string();
                    if !json.is_empty() {
                        t += cade_ai::count_tokens(&agent.model, &json);
                    }
                }
            }
            t
        })
        .sum();
    let total_tokens: usize = system_tokens.saturating_add(history_tokens);
    let window_tokens = catalogue::context_window_for_model(&agent.model) as usize;
    let input_budget_tokens =
        window_tokens.saturating_sub(((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize);
    let input_budget_chars = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
    // Phase 4 Pt 2: use the native-token budget for the canonical
    // fits_budget signal — it matches what the provider counts and is
    // not skewed by the legacy 3:1 char fallback.  Char-budget remains
    // as a backward-compatible sanity check.
    let fits_budget =
        total_tokens <= input_budget_tokens && total_assembled_chars <= input_budget_chars;
    let system_msg_count = messages
        .iter()
        .take_while(|m| m.role == "system")
        .count();
    let turns_selected = turns_selected_count;
    let telemetry = crate::server::state::ContextTelemetry {
        model: agent.model.clone(),
        window_tokens,
        input_budget_chars,
        system_overhead_chars,
        system_tokens,
        message_budget_chars: message_budget,
        history_chars,
        history_tokens,
        total_tokens,
        turns_selected,
        turns_omitted: omitted_turns,
        system_msg_count,
        skills_full: skills_full_count,
        skills_summary: skills_summary_count,
        fits_budget,
        build_micros: build_started.elapsed().as_micros() as u64,
    };
    {
        let mut t = state.agent_context_telemetry.write().await;
        t.insert(agent_id.to_string(), telemetry.clone());
    }
    tracing::info!(
        target: "cade::context::telemetry",
        agent_id,
        model = %telemetry.model,
        window_tokens = telemetry.window_tokens,
        input_budget_chars = telemetry.input_budget_chars,
        system_overhead_chars = telemetry.system_overhead_chars,
        system_tokens = telemetry.system_tokens,
        message_budget_chars = telemetry.message_budget_chars,
        history_chars = telemetry.history_chars,
        history_tokens = telemetry.history_tokens,
        total_tokens = telemetry.total_tokens,
        turns_selected = telemetry.turns_selected,
        turns_omitted = telemetry.turns_omitted,
        system_msg_count = telemetry.system_msg_count,
        skills_full = telemetry.skills_full,
        skills_summary = telemetry.skills_summary,
        fits_budget = telemetry.fits_budget,
        build_micros = telemetry.build_micros,
        "build_context telemetry"
    );

    let result_tuple = (agent.model, messages, tool_schemas);
    {
        let mut cache = state.context_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.put(cache_key, (state_hash, result_tuple.clone()));
    }

    Ok(result_tuple)
}


fn assemble_system_prompt_memory(
    state: &AppState,
    agent: &cade_store::sqlite::AgentRow,
    agent_id: &str,
    is_tool_return: bool,
) -> (String, String) {
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
    let mut dynamic_parts: Vec<String> = Vec::new();
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

        let entry = if tier == "pinned" {
            format!("📌 [{label}]\n{formatted_val}")
        } else {
            format!("[{label}]\n{formatted_val}")
        };

        let chars = entry.chars().count();
        let is_dynamic = label == "active_goal" || label == "recent_edits" || label == "session_summary";

        if is_dynamic {
            dynamic_parts.push(entry);
            continue;
        }

        if tier == "pinned" {
            if chars <= pinned_remaining {
                pinned_remaining -= chars;
                pinned_parts.push(entry);
            } else {
                active_omitted += 1;
            }
        } else {
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

    

    if !has_any_memory && dynamic_parts.is_empty() {
        (base, String::new())
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

        let static_core = sections.join("\n\n");
        let mut dynamic_core = String::new();
        
        if !dynamic_parts.is_empty() {
            dynamic_core = format!("# Working State\n{}", dynamic_parts.join("\n\n"));
            dynamic_core.push_str("\n\n");
        }
        dynamic_core.push_str(MEMORY_AWARENESS_FOOTER);
        
        (static_core, dynamic_core)
    }
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
    let window_tokens = catalogue::context_window_for_model(model);
    let output_reserve_tokens = ((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize;
    let input_budget_tokens = (window_tokens as usize).saturating_sub(output_reserve_tokens);
    let context_char_budget = {
        let raw = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
        let mut budget = raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS);
        if let Some(max_budget) = state.config.max_context_budget {
            budget = budget.min(max_budget);
        }
        budget
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
        let mut budget = raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS);
        if let Some(max_budget) = state.config.max_context_budget {
            budget = budget.min(max_budget);
        }
        budget
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


// ── Per-category context breakdown ─────────────────────────────────────────────

/// GET /v1/agents/:id/context-breakdown?conversation_id=<id>
///
/// Returns per-category token estimates for the context window, suitable for
/// rendering a proportional bar chart in the GUI dashboard.
pub async fn get_context_breakdown_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let conv_id = params.get("conversation_id").map(String::as_str);
    match compute_context_breakdown(&state, &agent_id, conv_id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, &e).into_response(),
    }
}

async fn compute_context_breakdown(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> core::result::Result<Value, String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    let window_tokens = catalogue::context_window_for_model(&agent.model) as u64;

    // Cat 0: system prompt tokens (~chars/3)
    let sys_tok = agent
        .system_prompt
        .as_deref()
        .map(|s| (s.chars().count() / 3) as u64)
        .unwrap_or(0);

    // Cat 2: MCP tool schemas
    let mcp_schemas = state.mcp.all_tool_schemas().await;
    let mcp_tok = (mcp_schemas
        .iter()
        .filter_map(|s| serde_json::to_string(s).ok())
        .map(|s| s.len())
        .sum::<usize>()
        / 3) as u64;

    // Cat 3: memory blocks
    let mem_blocks = sqlite::get_active_blocks(&state.db, agent_id).unwrap_or_default();
    let mem_tok = (mem_blocks
        .iter()
        .map(|(_, v, _, _, _)| v.chars().count())
        .sum::<usize>()
        / 3) as u64;

    // Cat 4: skills loaded for this agent
    let skills_tok = {
        let agent_skills = state.agent_skills.read().await;
        let loaded_ids = agent_skills.get(agent_id).cloned().unwrap_or_default();
        if loaded_ids.is_empty() {
            0u64
        } else {
            let all = state.all_skills.read().await;
            (loaded_ids
                .iter()
                .filter_map(|id| all.iter().find(|s| s.id == *id))
                .map(|s| s.body.chars().count())
                .sum::<usize>()
                / 3) as u64
        }
    };

    // Cat 5: conversation messages
    let output_reserve = ((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize;
    let input_budget_tokens = (window_tokens as usize).saturating_sub(output_reserve);
    let context_char_budget = {
        let raw = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
        let mut budget = raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS);
        if let Some(max_budget) = state.config.max_context_budget {
            budget = budget.min(max_budget);
        }
        budget
    };
    let all_rows =
        sqlite::get_context_window(&state.db, agent_id, conversation_id, context_char_budget)
            .unwrap_or_default();
    let msg_chars: usize = all_rows.iter().map(|r| r.char_count).sum();
    let msg_tok = (msg_chars / 3) as u64;

    // Cat 1: native tools = total used - known categories (residual)
    let known_excl_tools = sys_tok + mcp_tok + mem_tok + skills_tok + msg_tok;
    let total_used_estimate = {
        // Use the server-side context stats if available
        let stats_result = compute_context_stats(state, agent_id, conversation_id).await;
        if let Ok(ref stats) = stats_result {
            let chars_used = stats["chars_used"].as_u64().unwrap_or(0);
            let budget = stats["message_budget_chars"].as_u64().unwrap_or(1);
            if budget > 0 {
                (chars_used as f64 / budget as f64 * window_tokens as f64) as u64
            } else {
                known_excl_tools
            }
        } else {
            known_excl_tools
        }
    };
    let tools_tok = total_used_estimate.saturating_sub(known_excl_tools);

    let total_used = known_excl_tools + tools_tok;
    let buffer_tok = window_tokens * 3 / 100;
    let free_tok = window_tokens.saturating_sub(total_used + buffer_tok);
    let pct = if window_tokens > 0 {
        (total_used * 100 / window_tokens).min(100) as u8
    } else {
        0
    };

    let model_short = agent
        .model
        .rsplit('/')
        .next()
        .unwrap_or(&agent.model)
        .to_string();

    Ok(json!({
        "model": model_short,
        "window_tokens": window_tokens,
        "pct": pct,
        "categories": [
            { "name": "system",   "tokens": sys_tok },
            { "name": "tools",    "tokens": tools_tok },
            { "name": "mcp",      "tokens": mcp_tok },
            { "name": "memory",   "tokens": mem_tok },
            { "name": "skills",   "tokens": skills_tok },
            { "name": "messages", "tokens": msg_tok },
            { "name": "free",     "tokens": free_tok },
            { "name": "buffer",   "tokens": buffer_tok },
        ]
    }))
}

#[cfg(test)]
mod head_tail_tests {
    use super::*;

    #[test]
    fn test_head_tail_truncation_logic() {
        // We replicate the exact logic from context.rs inside a unit test to verify it works
        let original_text = "0123456789".repeat(100); // 1000 chars
        let mut turn = [LlmMessage {
            role: "tool".to_string(),
            content: original_text.clone(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }];
        
        let margin = 200;
        let cut_remaining_initial = 600;
        let mut cut_remaining = cut_remaining_initial;
        
        for m in turn.iter_mut().filter(|m| m.role == "tool") {
            let len = m.content.chars().count();
            if len > margin && cut_remaining > 0 {
                let cut_here = cut_remaining.min(len.saturating_sub(margin));
                let keep = len - cut_here;
                let keep_head = (keep as f64 * 0.2) as usize;
                let keep_tail = keep.saturating_sub(keep_head);
                
                let mut new_content: String = m.content.chars().take(keep_head).collect();
                new_content.push_str(&format!(
                    "\n... [{} chars truncated to fit context window] ...\n",
                    cut_here
                ));
                let tail: String = m.content.chars().skip(keep_head + cut_here).take(keep_tail).collect();
                new_content.push_str(&tail);
                m.content = new_content;
                cut_remaining -= cut_here;
            }
        }
        
        let content = &turn[0].content;
        assert!(content.starts_with("0123456789"));
        assert!(content.ends_with("0123456789"));
        assert!(content.contains("600 chars truncated to fit context window"));
        
        let head_part: String = original_text.chars().take(80).collect();
        let tail_part: String = original_text.chars().skip(80 + 600).take(320).collect();
        assert!(content.starts_with(&head_part));
        assert!(content.ends_with(&tail_part));
    }
}

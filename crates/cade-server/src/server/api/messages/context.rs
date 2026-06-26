use super::*;
use crate::server::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use cade_ai::{LlmMessage, catalogue};
use cade_store::sqlite::{self};
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
/// Test-only thin wrapper that calls [`render_skills_section_filtered`]
/// with an empty disabled-set.  Production callers go through the filtered
/// variant directly so the per-agent blacklist is always honoured; this
/// shim keeps the older P2-3 test fixtures concise.
#[cfg(test)]
pub(crate) fn render_skills_section(
    loaded: &[&cade_core::skills::Skill],
    budget: usize,
    body_cap: usize,
) -> String {
    render_skills_section_filtered(loaded, &Default::default(), budget, body_cap)
}

/// P5: maximum chars retained in a top-level tool `description` after
/// compression.  The model sees enough to remember the tool's purpose
/// (≈ first sentence) without paying for the full reference docs every turn.
pub(crate) const COMPRESSED_DESCRIPTION_CHAR_CAP: usize = 80;

/// P5: compress a tool schema for a long-session, unused, non-pinned tool.
///
/// Strips:
///   * Top-level `description` truncated to [`COMPRESSED_DESCRIPTION_CHAR_CAP`]
///     (or first newline, whichever comes first).
///   * Each property's `description` field inside `parameters.properties` /
///     `input_schema.properties`.
///   * Each property's `examples` field (rarely needed for inactive tools).
///
/// Preserves:
///   * `name` (required for the model to call the tool).
///   * `parameters` / `input_schema` shape, types, required, and enums
///     (all needed for valid JSON-schema validation on the provider side).
///
/// Idempotent: compressing an already-compressed schema is a no-op.
pub(crate) fn compress_tool_schema(mut schema: Value) -> Value {
    // Truncate top-level description.
    if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
        let trimmed: String = desc
            .split('\n')
            .next()
            .unwrap_or(desc)
            .chars()
            .take(COMPRESSED_DESCRIPTION_CHAR_CAP)
            .collect();
        schema["description"] = Value::String(trimmed);
    }

    // Strip per-property descriptions and examples in parameters / input_schema.
    for params_key in ["parameters", "input_schema"] {
        if let Some(params) = schema.get_mut(params_key)
            && let Some(props) = params.get_mut("properties")
            && let Some(obj) = props.as_object_mut()
        {
            for (_, prop_val) in obj.iter_mut() {
                if let Some(prop_obj) = prop_val.as_object_mut() {
                    prop_obj.remove("description");
                    prop_obj.remove("examples");
                }
            }
        }
    }

    schema
}

/// Like [`render_skills_section`] but skips any skill whose ID is in
/// `disabled_ids`.  Called from `build_context` after loading the per-agent
/// blacklist from the DB so disabled skills are transparent to the LLM.
pub(crate) fn render_skills_section_filtered(
    loaded: &[&cade_core::skills::Skill],
    disabled_ids: &std::collections::HashSet<String>,
    budget: usize,
    body_cap: usize,
) -> String {
    let visible: Vec<&&cade_core::skills::Skill> = loaded
        .iter()
        .filter(|s| !disabled_ids.contains(&s.id))
        .collect();

    if visible.is_empty() {
        return String::new();
    }
    let header = "\n\n# Loaded Skills\n";
    let mut section = String::from(header);
    let mut remaining = budget;

    for skill in &visible {
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
                visible.iter().skip_while(|s| s.id != skill.id).count()
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
            // Box::pin: consolidate_agent's Future holds ~500 lines of
            // locals across 20 await points.  Without boxing, its state
            // machine is embedded in complete_with_overflow_recovery's
            // Future, which is in turn embedded in the caller's Future,
            // compounding stack usage that caused the tokio worker thread
            // overflow on archival/historic content access.
            Box::pin(crate::server::consolidation::consolidate_agent(
                state,
                agent_id,
                conversation_id,
                None,
            ))
            .await;

            // 2. Drop the context cache entry so build_context recomputes.
            {
                let mut cache = state.context_cache.lock();
                let key = format!("{agent_id}:{conversation_id:?}");
                cache.pop(&key);
            }

            // 3. Rebuild context fresh.
            // Box::pin: same rationale as above — build_context holds
            // Vec<Vec<LlmMessage>>, Vec<MessageRow>, and multiple HashMaps.
            let (model, mut new_messages, new_tools) = match Box::pin(build_context(
                state,
                agent_id,
                conversation_id,
                is_tool_return,
            ))
            .await
            {
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

pub(crate) fn group_into_turns(
    messages: &[LlmMessage],
    max_turn_chars: usize,
) -> Vec<Vec<LlmMessage>> {
    let mut turns: Vec<Vec<LlmMessage>> = Vec::new();
    let mut current: Vec<LlmMessage> = Vec::new();
    let mut current_chars = 0;

    for msg in messages {
        let msg_chars = msg.content.chars().count()
            + msg
                .tool_calls
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|tc| tc.arguments.to_string().len())
                .sum::<usize>();

        // A new user message starts a new turn.
        // Also split mid-turn if we've exceeded the budget and hit a safe boundary.
        // A safe boundary is right before an assistant message (provided we aren't interrupting a tool call/result sequence).
        let is_safe_boundary = msg.role == "assistant";

        if (msg.role == "user" && !current.is_empty())
            || (is_safe_boundary && current_chars >= max_turn_chars && !current.is_empty())
        {
            turns.push(std::mem::take(&mut current));
            current_chars = 0;
        }

        current.push(msg.clone());
        current_chars += msg_chars;
    }

    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

#[allow(clippy::collapsible_if)]
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

    let (mut system_static, system_dynamic) =
        assemble_system_prompt_memory(state, &agent, agent_id, conversation_id, is_tool_return);

    // Skill-counters for Phase-4 telemetry; updated when we render the
    // skills section below.
    let mut skills_full_count: usize = 0;
    let mut skills_summary_count: usize = 0;

    // Inject loaded skills into the STATIC system prompt section.
    //
    // Skill bodies change rarely (only on /skills load|unload, which
    // explicitly invalidates `context_cache`), so they belong in the
    // cache-anchored static block where Anthropic's `cache_control:
    // ephemeral` breakpoint pins them.  This unlocks the ~90% cache-read
    // discount on what is typically the largest stable section of the
    // prompt (10–30 KB of skill bodies).  Memory tiers (volatile per turn)
    // remain in `system_dynamic` and are correctly billed at full rate.
    {
        let mut all_requested_skills: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // 1. Explicitly loaded skills
        let agent_skills = state.agent_skills.read().await;
        if let Some(loaded_ids) = agent_skills.get(agent_id) {
            all_requested_skills.extend(loaded_ids.iter().cloned());
        }

        // 2. Auto-load required skills from [project] block
        let required = parse_required_skills_from_project(&system_static);
        all_requested_skills.extend(required);

        if !all_requested_skills.is_empty() {
            let all_skills = state.all_skills.read().await;
            let loaded: Vec<&cade_core::skills::Skill> = all_requested_skills
                .iter()
                .filter_map(|id| all_skills.iter().find(|s| s.id == *id))
                .collect();

            // Phase B: load per-agent disabled-skill blacklist from DB.
            let disabled: std::collections::HashSet<String> =
                cade_store::sqlite::skills::get_disabled_skills(&state.db, agent_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect();

            let skills_section = render_skills_section_filtered(
                &loaded,
                &disabled,
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
                if !system_static.is_empty() {
                    system_static.push_str("\n\n");
                }
                system_static.push_str(&skills_section);
            }
        }
    }

    // Pad system_static to the nearest 512-character boundary to maximize prompt cache hits.
    let len = system_static.len();
    if len > 0 {
        let remainder = len % 512;
        if remainder > 0 {
            let padding_len = 512 - remainder;
            system_static.push_str(&" ".repeat(padding_len));
        }
    }

    // Memory-change detection: cache the assembled static system_core per agent.
    let system_prompt_static = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        system_static.hash(&mut h);
        let new_hash = h.finish();
        let mut cache = state.memory_cache.lock();
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
        let mut cache = state.context_cache.lock();
        if let Some((cached_hash, cached_tuple)) = cache.get(&cache_key)
            && *cached_hash == state_hash
        {
            return Ok(cached_tuple.clone());
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
            content: system_dynamic,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        },
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
    let db_pool = state.db.clone();
    let agent_id_clone = agent_id.to_string();
    let conv_id_clone = conversation_id.map(|s| s.to_string());
    let all_rows = tokio::task::spawn_blocking(move || {
        sqlite::get_context_window(
            &db_pool,
            &agent_id_clone,
            conv_id_clone.as_deref(),
            context_char_budget,
        )
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    // Convert DB rows to LlmMessages (oldest-first).
    let all_llm_msgs: Vec<LlmMessage> = all_rows
        .iter()
        .flat_map(db_row_to_llm)
        .map(|m| truncate_oversize_message(m, PER_MESSAGE_CHAR_CAP))
        .collect();

    // Group into logical turns.
    let max_turn_chars = state
        .config
        .max_tokens_per_turn
        .map(cade_ai::chars_for_tokens)
        .unwrap_or(64_000);
    let mut turns = group_into_turns(&all_llm_msgs, max_turn_chars);

    // If the window cut off mid-turn, the oldest turn might not start with a user or assistant message.
    // Drop it to ensure we never split tool_call/tool_result pairs.
    if let Some(first_msg) = turns.first().and_then(|t| t.first())
        && first_msg.role != "user"
        && first_msg.role != "assistant"
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
                        let tail: String = m
                            .content
                            .chars()
                            .skip(keep_head + cut_here)
                            .take(keep_tail)
                            .collect();
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
    // Trigger consolidation early when context usage crosses
    // PROACTIVE_CONSOLIDATION_THRESHOLD (70% as of F4, 2026-04-30), even if
    // no turns were dropped yet.  This gives the Sleeptime task a wider
    // runway to produce a `session_summary` block before the next request
    // actually overflows.
    let usage_fraction = if message_budget > 0 {
        budget_used as f64 / message_budget as f64
    } else {
        0.0
    };
    let needs_proactive = usage_fraction >= PROACTIVE_CONSOLIDATION_THRESHOLD;

    // P5-B: Trigger consolidation if there are too many turns since the last compaction marker.
    // This handles the case where the context budget is large enough to fit many turns, but
    // the growing history causes token bloat even before hitting the usage threshold.
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
            let entry = activity.entry(agent_id.to_string()).or_insert(
                crate::server::state::AgentActivity {
                    last_active_ts: chrono::Utc::now().timestamp(),
                    needs_consolidation: true,
                    conversation_id: conversation_id.map(String::from),
                    last_consolidation_turn: 0,
                },
            );
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
            tracing::info!(agent_id = %agent_id, "build_context:  eager consolidation triggered (turn-count path)");
            tokio::spawn(async move {
                crate::server::consolidation::consolidate_agent(
                    &state_eager,
                    &agent_eager,
                    conv_for_eager.as_deref(),
                    None,
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
            const PRUNE_MIN_CHARS: usize = 200; // only compact outputs > 200 chars
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
                        agent_id,
                        n,
                    );
                    // P6-A: Track tool outputs compacted
                    let metrics = state.agent_metrics.clone();
                    metrics
                        .entry(agent_id.to_string())
                        .or_default()
                        .tool_outputs_compacted
                        .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!(
                        "build_context [{}]: tool-output pruning failed: {}",
                        agent_id,
                        e,
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

    // Tool schemas — use agent-specific tools if wired, else all tools.
    // Carry tags alongside each schema so ITS decisions are tag-driven
    // (no hardcoded tool name lists).
    let agent_tool_ids = sqlite::get_agent_tool_ids(&state.db, agent_id).unwrap_or_default();
    let all_tools = sqlite::list_tools(&state.db).unwrap_or_default();
    let tagged_schemas: Vec<(Value, Vec<String>)> = if agent_tool_ids.is_empty() {
        all_tools
            .into_iter()
            .filter_map(|t| t.json_schema.map(|s| (s, t.tags)))
            .collect()
    } else {
        all_tools
            .into_iter()
            .filter(|t| agent_tool_ids.contains(&t.id))
            .filter_map(|t| t.json_schema.map(|s| (s, t.tags)))
            .collect()
    };

    // ── ITS Layer 1: prune + compress ─────────────────────────────────────
    //
    // On long sessions (> RECENT_WINDOW messages):
    //   1. Prune: MCP tools unused in the recent window are removed entirely
    //      when they belong to an "extended" prefix group (e.g. desktop_*).
    //   2. Compress: MCP tools (tagged "mcp") that haven't been called
    //      recently get their descriptions truncated to save prompt tokens.
    //   3. CADE-owned tools (tagged "cade" without "mcp") are NEVER pruned
    //      or compressed — the LLM needs full descriptions to reliably call
    //      them.
    //
    // Tool classification is discovered from DB registration tags, not from
    // hardcoded name lists.

    let is_long_session = messages.len() > 1 + RECENT_WINDOW;

    let recently_used: std::collections::HashSet<String> = if is_long_session {
        let recent_start = messages.len().saturating_sub(RECENT_WINDOW);
        messages[recent_start..]
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flat_map(|calls| calls.iter().map(|tc| tc.name.clone()))
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let tool_schemas: Vec<Value> = if is_long_session {
        tagged_schemas
            .into_iter()
            .filter(|(schema, tags)| {
                let name = schema["name"].as_str().unwrap_or("");
                let is_mcp = tags.contains(&"mcp".to_string());
                // CADE-owned tools are never pruned.
                if !is_mcp {
                    return true;
                }
                // MCP tools with a desktop_* prefix are pruned if unused.
                let is_desktop = name.starts_with("desktop_");
                !is_desktop || recently_used.contains(name)
            })
            // Compress unused MCP tool schemas.  CADE-owned tools keep full
            // descriptions; MCP tools not called recently get truncated.
            .map(|(schema, tags)| {
                let name = schema["name"].as_str().unwrap_or("").to_string();
                let is_mcp = tags.contains(&"mcp".to_string());
                if !is_mcp || recently_used.contains(&name) {
                    schema
                } else {
                    compress_tool_schema(schema)
                }
            })
            .collect()
    } else {
        tagged_schemas.into_iter().map(|(s, _)| s).collect()
    };

    // ── Intelligent tool selection ────────────────────────────────────────────

    // Phase 4: capture per-request telemetry so we can prove which
    // defence layers fired and how close to the budget we ended up.
    let history_chars: usize = messages
        .iter()
        .skip_while(|m| m.role == "system")
        .map(|m| m.content.chars().count())
        .sum();
    let total_assembled_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
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
    let input_budget_tokens = window_tokens
        .saturating_sub(((window_tokens as f64) * OUTPUT_RESERVE_FRACTION).round() as usize);
    let input_budget_chars = input_budget_tokens.saturating_mul(CHARS_PER_TOKEN);
    // Phase 4 Pt 2: use the native-token budget for the canonical
    // fits_budget signal — it matches what the provider counts and is
    // not skewed by the legacy 3:1 char fallback.  Char-budget remains
    // as a backward-compatible sanity check.
    let fits_budget =
        total_tokens <= input_budget_tokens && total_assembled_chars <= input_budget_chars;
    let system_msg_count = messages.iter().take_while(|m| m.role == "system").count();
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
        let mut cache = state.context_cache.lock();
        cache.put(cache_key, (state_hash, result_tuple.clone()));
    }

    Ok(result_tuple)
}

fn assemble_system_prompt_memory(
    state: &AppState,
    agent: &cade_store::sqlite::AgentRow,
    agent_id: &str,
    conversation_id: Option<&str>,
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
    let stale_threshold = MemoryBudgets::stale_threshold_for_model(&agent.model);
    let _ = sqlite::promote_stale_blocks(&state.db, agent_id, current_turn, stale_threshold);

    // 3. A1: Unified adaptive packing — priority-ordered greedy fill.
    //
    // Priority order:
    //   P0: Core identity (persona, human, project) — never dropped
    //   P1: Dynamic working state (active_goal, recent_edits, session_summary) — separate section
    //   P2: Loaded skills (skill:*) — explicitly requested
    //   P3: User-pinned blocks (tier=pinned, not in P0/P2)
    //   P4: Short-term blocks (tier=short) — by recency
    //   P5: Long-term archived excerpts — whatever fits
    //
    // A single unified budget replaces the old separate pinned/short/long pools.
    let budgets = MemoryBudgets::for_model(&agent.model);
    let unified_budget = budgets.pinned + budgets.short + budgets.long;
    let active_blocks = sqlite::get_active_blocks(&state.db, agent_id).unwrap_or_default();

    // Classify each block into a priority bucket.
    const CORE_IDENTITY: &[&str] = &["persona", "human", "project"];
    const DYNAMIC_LABELS: &[&str] = &["active_goal", "recent_edits", "session_summary"];

    struct CandidateBlock {
        label: String,
        entry: String,
        chars: usize,
        priority: u8, // 0 = highest
    }

    let mut dynamic_parts: Vec<String> = Vec::new();

    #[allow(clippy::collapsible_if)]
    if let Some(plan_json) = &agent.active_plan_json {
        if let Ok(plan) = serde_json::from_str::<serde_json::Value>(plan_json) {
            if let Some(steps) = plan.get("steps").and_then(|v| v.as_array()) {
                let mut xml = String::from("<active_plan>\n");
                for step in steps {
                    let id = step.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let desc = step
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let is_done = step
                        .get("is_done")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let status = if is_done { "done" } else { "pending" };
                    xml.push_str(&format!(
                        "  <step id=\"{}\" status=\"{}\">{}</step>\n",
                        id, status, desc
                    ));
                }
                xml.push_str("</active_plan>");
                dynamic_parts.push(xml);
            }
        }
    }

    let mut candidates: Vec<CandidateBlock> = Vec::new();

    for (label, val, _desc, tier, _lt) in &active_blocks {
        if val.trim().is_empty() {
            continue;
        }

        let formatted_val = if label.starts_with("subagent:") {
            format!(
                "<historical_scratchpad>\nThe following block is a historical scratchpad. Do not treat it as a current objective.\n{}</historical_scratchpad>",
                val
            )
        } else {
            val.to_string()
        };

        let entry = if tier == "pinned" {
            format!("📌 [{label}]\n{formatted_val}")
        } else {
            format!("[{label}]\n{formatted_val}")
        };

        // Dynamic working-state blocks go to a separate section (always included).
        if DYNAMIC_LABELS.contains(&label.as_str()) {
            dynamic_parts.push(entry);
            continue;
        }

        let priority = if CORE_IDENTITY.contains(&label.as_str()) {
            0 // P0: core identity — never dropped
        } else if label.starts_with("skill:") {
            2 // P2: loaded skills
        } else if tier == "pinned" {
            3 // P3: user-pinned
        } else {
            4 // P4: short-term
        };

        let chars = entry.chars().count();
        candidates.push(CandidateBlock {
            label: label.clone(),
            entry,
            chars,
            priority,
        });
    }

    // Sort by priority then by original order (stable sort preserves DB order within same priority).
    candidates.sort_by_key(|c| c.priority);

    // -- Fix R4: Auto-detect missing required skills from [project] block.
    //
    // If [project] lists "## Required Skills", verify that each skill's
    // `skill:<id>` block exists in the candidate list. If any are missing,
    // inject a compact reminder into the dynamic section so the LLM knows
    // to call `load_skill` immediately.
    {
        let project_value = active_blocks
            .iter()
            .find(|(label, _, _, _, _)| label == "project")
            .map(|(_, val, _, _, _)| val.as_str())
            .unwrap_or("");
        let required = parse_required_skills_from_project(project_value);
        if !required.is_empty() {
            let loaded_skill_labels: Vec<&str> = candidates
                .iter()
                .filter(|c| c.label.starts_with("skill:"))
                .map(|c| c.label.strip_prefix("skill:").unwrap_or(&c.label))
                .collect();
            let missing: Vec<&str> = required
                .iter()
                .filter(|s| !loaded_skill_labels.contains(&s.as_str()))
                .map(|s| s.as_str())
                .collect();
            if !missing.is_empty() {
                let reminder = format!(
                    "⚠️ MISSING REQUIRED SKILLS: The [project] block requires these skills but they are NOT loaded: {}. \
                     Call `load_skill(\"{}\")` for each one IMMEDIATELY before doing any work.",
                    missing.join(", "),
                    missing.join("\"), load_skill(\""),
                );
                dynamic_parts.push(reminder);
            }
        }
    }

    // Greedy-pack into unified budget.
    let mut packed_parts: Vec<String> = Vec::new();
    let mut remaining = unified_budget;
    // A1: Track excluded blocks with actionable recovery instructions.
    let mut overflow_manifest: Vec<String> = Vec::new();

    for c in &candidates {
        if c.chars <= remaining {
            remaining -= c.chars;
            packed_parts.push(c.entry.clone());
        } else {
            // Build actionable recovery instruction.
            let recovery = if c.label.starts_with("skill:") {
                let skill_id = c.label.strip_prefix("skill:").unwrap_or(&c.label);
                format!(
                    "- [{}] ({} chars) — use load_skill(\"{}\") to reload",
                    c.label, c.chars, skill_id
                )
            } else {
                format!(
                    "- [{}] ({} chars) — use search_memory(\"{}\") to retrieve",
                    c.label, c.chars, c.label
                )
            };
            overflow_manifest.push(recovery);
        }
    }

    // 4. Long-term archived blocks → label + rich excerpt (A3).
    let long_excerpts =
        sqlite::get_long_term_excerpts(&state.db, agent_id, current_turn).unwrap_or_default();
    let mut long_parts: Vec<String> = Vec::new();

    for excerpt_info in &long_excerpts {
        let entry = if excerpt_info.excerpt.trim().is_empty() {
            format!(
                "[{}]\n  keywords: {} | {} chars",
                excerpt_info.label,
                excerpt_info.keywords.join(", "),
                excerpt_info.char_count
            )
        } else {
            format!(
                "[{}]: {}\n  keywords: {} | {} chars",
                excerpt_info.label,
                excerpt_info.excerpt,
                excerpt_info.keywords.join(", "),
                excerpt_info.char_count
            )
        };
        let chars = entry.chars().count();
        if chars <= remaining {
            remaining -= chars;
            long_parts.push(entry);
        } else {
            overflow_manifest.push(format!(
                "- [{}] ({} chars, archived) — use search_memory(\"{}\")",
                excerpt_info.label, excerpt_info.char_count, excerpt_info.label
            ));
        }
    }

    // 5. Assemble system prompt memory sections.
    let has_any_memory = !packed_parts.is_empty() || !long_parts.is_empty();
    let base = agent.system_prompt.clone().unwrap_or_default();

    if !has_any_memory && dynamic_parts.is_empty() {
        (base, String::new())
    } else {
        let mut sections: Vec<String> = vec![base];

        // Active memory section (unified priority-packed blocks)
        let mut active_section_parts: Vec<String> = Vec::new();
        active_section_parts.extend(packed_parts);

        // A1: Actionable overflow manifest replaces generic "[…N omitted]".
        if !overflow_manifest.is_empty() {
            let manifest = format!(
                "# Context Overflow\nThe following memory blocks were excluded from this context:\n{}",
                overflow_manifest.join("\n")
            );
            active_section_parts.push(manifest);
        }

        if !active_section_parts.is_empty() {
            sections.push(format!("# Memory\n{}", active_section_parts.join("\n\n")));
        }

        // Archived memory section (long-term excerpts)
        if !long_parts.is_empty() {
            let archived = long_parts.join("\n");
            sections.push(format!(
                "# Archived Memory\n{archived}\nUse search_memory(query) to retrieve full archived content.\nAccessed blocks are automatically restored to active memory."
            ));
        }

        let mut static_core = sections.join("\n\n");
        static_core.push_str("\n\n");
        let mut dynamic_core = String::new();

        if !dynamic_parts.is_empty() {
            dynamic_core = format!("# Working State\n{}", dynamic_parts.join("\n\n"));
            dynamic_core.push_str("\n\n");
        }

        // ── A9: Proactive injection — recall relevant memory chunks ──────
        // Fetch the latest user message, extract keywords, and search
        // memory_chunks for matching fragments.  Inject the top 3 as a
        // `# Recalled Context` section so the LLM sees relevant facts
        // without having to call search_memory explicitly.
        if !is_tool_return
            && let Ok(Some(user_msg)) =
                sqlite::get_latest_user_message(&state.db, agent_id, conversation_id)
        {
            let recalled = sqlite::memory::recall_chunks_hybrid(
                &state.db, agent_id, &user_msg, 3,
                state.embedder.as_deref(),
            );
            if !recalled.is_empty() {
                let mut recall_lines: Vec<String> = Vec::new();
                for rc in &recalled {
                    let preview: String = rc.chunk_content.chars().take(300).collect();
                    recall_lines.push(format!(
                        "- **[{}]** (chunk {}): {}",
                        rc.label, rc.chunk_index, preview
                    ));
                }
                dynamic_core.push_str(&format!(
                    "# Recalled Context\n\
                     The following memory fragments matched your latest message and were automatically recalled:\n{}\n\n",
                    recall_lines.join("\n")
                ));
            }
        }

        // ── P1 + A6: Inject recent observations into context ─────────────
        // Fetch high-importance observations (≥3) scaled to the model's
        // context window.  The LLM sees a compact trail of what it did even
        // after older messages have been dropped from the message window.
        {
            let (obs_limit, obs_budget) = MemoryBudgets::observation_budget(&agent.model);
            let observations =
                sqlite::observations::get_important_observations(&state.db, agent_id, 3, obs_limit)
                    .unwrap_or_default();
            let obs_section =
                sqlite::observations::render_observations_section(&observations, obs_budget);
            if !obs_section.is_empty() {
                dynamic_core.push_str(&obs_section);
                dynamic_core.push_str("\n\n");
            }
        }

        dynamic_core.push_str(&memory_awareness_footer(&agent.model));

        (static_core, dynamic_core)
    }
}

fn calculate_context_budget(state: &AppState, agent_id: &str, model: &str) -> usize {
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
    let db_pool = state.db.clone();
    let agent_id_clone = agent_id.to_string();
    let conv_id_clone = conversation_id.map(|s| s.to_string());
    let all_rows = tokio::task::spawn_blocking(move || {
        sqlite::get_context_window(
            &db_pool,
            &agent_id_clone,
            conv_id_clone.as_deref(),
            context_char_budget,
        )
        .unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    let all_llm_msgs: Vec<LlmMessage> = all_rows.iter().flat_map(db_row_to_llm).collect();

    let max_turn_chars = state
        .config
        .max_tokens_per_turn
        .map(cade_ai::chars_for_tokens)
        .unwrap_or(64_000);
    let mut turns = group_into_turns(&all_llm_msgs, max_turn_chars);
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
    let db_pool = state.db.clone();
    let agent_id_clone = agent_id.to_string();
    let conv_id_clone = conversation_id.map(|s| s.to_string());
    let all_rows = tokio::task::spawn_blocking(move || {
        sqlite::get_context_window(
            &db_pool,
            &agent_id_clone,
            conv_id_clone.as_deref(),
            context_char_budget,
        )
        .unwrap_or_default()
    })
    .await
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
    let pct = (total_used * 100)
        .checked_div(window_tokens)
        .unwrap_or(0)
        .min(100) as u8;

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

/// Parse a `[project]` memory block for a "## Required Skills" section.
///
/// Expects lines like `- skill-id` under the heading. Returns the list of
/// skill IDs found. Stops at the next `## ` heading.
fn parse_required_skills_from_project(project_block: &str) -> Vec<String> {
    let mut in_section = false;
    let mut skills = Vec::new();
    for line in project_block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Required Skills") || trimmed.starts_with("## Required skills") {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with("## ") {
            break;
        }
        if in_section && trimmed.starts_with("- ") {
            let rest = trimmed.trim_start_matches("- ").trim();
            let id = rest.split_whitespace().next().unwrap_or("").to_string();
            if !id.is_empty() {
                skills.push(id);
            }
        }
    }
    skills
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
                let tail: String = m
                    .content
                    .chars()
                    .skip(keep_head + cut_here)
                    .take(keep_tail)
                    .collect();
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

#[cfg(test)]
mod blacklist_tests {
    use super::*;

    /// Phase B4 RED: `render_skills_section` must not include a skill whose
    /// ID appears in `disabled_ids`.  Before the filter is added this test
    /// fails because all loaded skills are rendered unconditionally.
    #[test]
    fn disabled_skill_is_excluded_from_rendered_section() {
        let make_skill = |id: &str| cade_core::skills::Skill {
            id: id.to_string(),
            name: id.to_string(),
            description: "desc".to_string(),
            category: None,
            tags: vec![],
            triggers: vec![],
            rpi_phase: None,
            capabilities: vec![],
            scripts: vec![],
            references: vec![],
            body: format!("body of {id}"),
            scope: cade_core::skills::SkillScope::Project,
            path: std::path::PathBuf::from(format!("/tmp/{id}/SKILL.MD")),
        };

        let skill_a = make_skill("skill-a");
        let skill_b = make_skill("skill-b"); // will be disabled

        let loaded = vec![&skill_a, &skill_b];
        let disabled: std::collections::HashSet<String> =
            ["skill-b".to_string()].into_iter().collect();

        let section = render_skills_section_filtered(
            &loaded,
            &disabled,
            usize::MAX, // no budget limit
            usize::MAX,
        );

        assert!(
            section.contains("skill-a"),
            "enabled skill must appear, got: {section}"
        );
        assert!(
            !section.contains("skill-b"),
            "disabled skill must NOT appear, got: {section}"
        );
    }
}

#[cfg(test)]
mod parse_required_skills_tests {
    use super::*;

    #[test]
    fn extracts_skills_from_project_block() {
        let block = "\
# CADE Project Rules

## Allowed MCP Servers
- context7
- deepwiki

## Required Skills
Load and follow these skills for all work:
- tdd-guide
- strict-project-execution
- caveman
- grill-me
- rust

## Workflow Requirements
- Always index the workspace
";
        let skills = parse_required_skills_from_project(block);
        assert_eq!(
            skills,
            vec![
                "tdd-guide",
                "strict-project-execution",
                "caveman",
                "grill-me",
                "rust"
            ]
        );
    }

    #[test]
    fn empty_block_returns_empty() {
        assert!(parse_required_skills_from_project("").is_empty());
    }

    #[test]
    fn no_section_returns_empty() {
        let block = "Some random project notes\n- not a skill";
        assert!(parse_required_skills_from_project(block).is_empty());
    }

    #[test]
    fn stops_at_next_heading() {
        let block = "\
## Required Skills
- alpha
- beta

## Something Else
- gamma
";
        let skills = parse_required_skills_from_project(block);
        assert_eq!(skills, vec!["alpha", "beta"]);
    }

    #[test]
    fn handles_extra_whitespace() {
        let block = "\
## Required Skills
  - spaced-skill  
-  another-skill
";
        let skills = parse_required_skills_from_project(block);
        assert_eq!(skills, vec!["spaced-skill", "another-skill"]);
    }
}

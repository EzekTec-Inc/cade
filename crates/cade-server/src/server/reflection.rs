use cade_ai::{CompletionRequest, LlmMessage};

use crate::server::state::AppState;
use cade_store::sqlite;

// region:    --- Tunables

const MAX_HISTORY_CHARS: usize = 18_000;
const REFLECTION_MAX_TOKENS: u32 = 1_000;
const MIN_MESSAGES_FOR_REFLECTION: usize = 6;

// endregion: --- Tunables

// region:    --- Public API

#[derive(Debug, Default)]
pub struct ReflectionResult {
    pub blocks_created: usize,
    pub blocks_updated: usize,
    pub summary: String,
    pub duration_ms: u128,
}

/// Run a reflection pass over an agent's recent conversation.
///
/// `focus` is an optional hint (e.g. "project conventions") that steers the
/// LLM toward specific knowledge categories.
pub async fn reflect_agent(
    state: &AppState,
    agent_id: &str,
    conv_id: Option<&str>,
    focus: Option<&str>,
    trigger: &str,
) -> ReflectionResult {
    let t0 = std::time::Instant::now();
    let mut result = ReflectionResult::default();

    // -- 1. Fetch recent messages
    // Use get_context_window to respect the compaction boundary. We don't want
    // to re-reflect on history that has already been compressed.
    let mut rows =
        sqlite::get_context_window(&state.db, agent_id, conv_id, 999_999).unwrap_or_default();

    // limit to most recent 200 just in case
    if rows.len() > 200 {
        rows = rows[rows.len() - 200..].to_vec();
    }

    if rows.len() < MIN_MESSAGES_FOR_REFLECTION {
        result.summary = "Not enough conversation history to reflect on yet.".to_string();
        return result;
    }

    // -- 2. Build a text summary of recent history (user + assistant turns only)
    let mut history_text = String::new();
    for row in &rows {
        let role = &row.role;
        if !matches!(role.as_str(), "user" | "assistant") {
            continue;
        }
        let text = row.content["content"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| {
                let raw = row.content.to_string();
                if raw.len() > 300 {
                    format!("{}…", &raw[..300])
                } else {
                    raw
                }
            });
        if text.trim().is_empty() {
            continue;
        }
        history_text.push_str(&format!("[{role}] {}\n", text.trim()));
        if history_text.len() >= MAX_HISTORY_CHARS {
            break;
        }
    }

    if history_text.trim().is_empty() {
        result.summary = "No text content to reflect on.".to_string();
        return result;
    }

    // -- 3. Fetch existing memory blocks to avoid duplication
    let existing = sqlite::get_memory_blocks(&state.db, agent_id).unwrap_or_default();
    let existing_labels: Vec<&str> = existing.iter().map(|(l, _, _)| l.as_str()).collect();
    let existing_summary = if existing_labels.is_empty() {
        "None yet.".to_string()
    } else {
        existing_labels.join(", ")
    };

    // -- 4. Build reflection prompt
    let focus_section = focus
        .map(|f| format!("\n\nFocus especially on: {f}"))
        .unwrap_or_default();
    let prompt = format!(
        "You are a memory extraction assistant for a stateful coding agent.\n\
         Analyse this conversation and extract NEW knowledge to persist.\n\
         Existing memory labels (do not duplicate): {existing_summary}\n\
         {focus_section}\n\n\
         For each new fact, output EXACTLY this JSON format on a separate line:\n\
         {{\"label\": \"snake_case_label\", \"value\": \"concise fact\", \"type\": \"<type>\"}}\n\n\
         Valid types: project_fact, user_pref, decision, constraint, convention, dependency, person, environment\n\n\
         Rules:\n\
         - Only extract PERSISTENT facts (not transient task steps)\n\
         - Keep values concise (≤200 chars)\n\
         - Use specific labels (not 'info' or 'note')\n\
         - Output 0–8 facts maximum\n\
         - Output ONLY the JSON lines, nothing else\n\n\
         CONVERSATION:\n{history_text}"
    );

    // -- 5. Call LLM
    let req = CompletionRequest {
        model: get_agent_model(state, agent_id),
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: REFLECTION_MAX_TOKENS,
        reasoning_effort: None,
    };

    let llm_output = match state.llm.complete(&req).await {
        Ok(r) => r.content.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(agent_id = %agent_id, "reflect_agent:  LLM failed: {e}");
            result.summary = format!("Reflection LLM error: {e}");
            return result;
        }
    };

    // -- 6. Parse JSON lines and upsert memory blocks
    let mut extracted: Vec<(String, String, String)> = Vec::new();
    for line in llm_output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let label = v["label"].as_str().unwrap_or("").trim().to_lowercase();
            let value = v["value"].as_str().unwrap_or("").trim().to_string();
            let mtype = v["type"].as_str().unwrap_or("generic").to_string();
            if label.is_empty() || value.is_empty() {
                continue;
            }
            // Validate label format: only alphanumeric + underscore
            if !label.chars().all(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            extracted.push((label, value, mtype));
        }
    }

    for (label, value, memory_type) in &extracted {
        let is_new = !existing.iter().any(|(l, _, _)| l == label);
        match sqlite::upsert_memory_block_typed(
            &state.db,
            agent_id,
            label,
            value,
            Some(&format!("Extracted by reflection ({trigger})")),
            None,
            Some(memory_type.as_str()),
            Some(0.9),
        ) {
            Ok(_) => {
                if is_new {
                    result.blocks_created += 1;
                } else {
                    result.blocks_updated += 1;
                }
            }
            Err(e) => tracing::warn!("reflect_agent: upsert '{label}': {e}"),
        }
    }

    result.summary = if extracted.is_empty() {
        "No new facts extracted from recent history.".to_string()
    } else {
        format!(
            "Extracted {} fact(s): {}",
            extracted.len(),
            extracted
                .iter()
                .map(|(l, _, _)| l.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    // -- 7. Log the reflection run
    let duration_ms = t0.elapsed().as_millis();
    result.duration_ms = duration_ms;
    let log_id = format!("rl-{}", uuid::Uuid::new_v4());
    let _ = sqlite::insert_reflection_log(
        &state.db,
        &log_id,
        agent_id,
        trigger,
        result.blocks_created,
        result.blocks_updated,
        &result.summary,
        duration_ms,
    );

    result
}

// endregion: --- Public API

// region:    --- Support

fn get_agent_model(state: &AppState, agent_id: &str) -> String {
    sqlite::get_agent(&state.db, agent_id)
        .ok()
        .flatten()
        .map(|a| a.model)
        .unwrap_or_else(|| "anthropic/claude-sonnet-4-5-20250929".to_string())
}

// endregion: --- Support

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
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let cid = conv_id.map(String::from);
    let rows_res = tokio::task::spawn_blocking(move || {
        sqlite::get_context_window(&db, &aid, cid.as_deref(), 999_999)
    })
    .await;
    let mut rows = rows_res.unwrap_or_else(|_| Ok(vec![])).unwrap_or_default();

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
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let existing_res =
        tokio::task::spawn_blocking(move || sqlite::get_memory_blocks(&db, &aid)).await;
    let existing = existing_res
        .unwrap_or_else(|_| Ok(vec![]))
        .unwrap_or_default();

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
    let model = match get_agent_model(state, agent_id).await {
        Ok(model) => model,
        Err(error) => {
            tracing::warn!(agent_id = %agent_id, "reflect_agent: model selection failed: {error}");
            result.summary = format!("Reflection model error: {error}");
            result.duration_ms = t0.elapsed().as_millis();
            return result;
        }
    };
    let max_tokens = REFLECTION_MAX_TOKENS.min(cade_ai::catalogue::max_tokens_for_model(&model));
    let req = CompletionRequest {
        model,
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: vec![],
        max_tokens,
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

    let db = state.db.clone();
    let aid = agent_id.to_string();
    let extracted_to_move = extracted.clone();
    let existing_to_move = existing.clone();
    let trigger_to_move = trigger.to_string();

    let loop_res = tokio::task::spawn_blocking(move || {
        let mut created = 0;
        let mut updated = 0;
        for (label, value, memory_type) in &extracted_to_move {
            let is_new = !existing_to_move.iter().any(|(l, _, _)| l == label);
            match sqlite::upsert_memory_block_typed(
                &db,
                &aid,
                label,
                value,
                Some(&format!("Extracted by reflection ({trigger_to_move})")),
                None,
                Some(memory_type.as_str()),
                Some(0.9),
            ) {
                Ok(_) => {
                    if is_new {
                        created += 1;
                    } else {
                        updated += 1;
                    }
                }
                Err(e) => tracing::warn!("reflect_agent: upsert '{label}': {e}"),
            }
        }
        (created, updated)
    })
    .await;

    let (created, updated) = loop_res.unwrap_or((0, 0));
    result.blocks_created = created;
    result.blocks_updated = updated;

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

    let db = state.db.clone();
    let log_id_to_move = log_id.clone();
    let aid = agent_id.to_string();
    let trigger_to_move = trigger.to_string();
    let summary_to_move = result.summary.clone();
    let _ = tokio::task::spawn_blocking(move || {
        sqlite::insert_reflection_log(
            &db,
            &log_id_to_move,
            &aid,
            &trigger_to_move,
            created,
            updated,
            &summary_to_move,
            duration_ms,
        )
    })
    .await;

    result
}

// endregion: --- Public API

// region:    --- Support

async fn get_agent_model(state: &AppState, agent_id: &str) -> Result<String, String> {
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let agent = tokio::task::spawn_blocking(move || sqlite::get_agent(&db, &aid))
        .await
        .map_err(|error| format!("Agent model lookup task failed: {error}"))?
        .map_err(|error| format!("Agent model lookup failed: {error}"))?;
    let model = agent
        .map(|agent| agent.model)
        .unwrap_or_else(|| state.config.default_model.clone());
    if model.trim().is_empty() {
        return Err("No agent or server default model configured".into());
    }
    state
        .llm
        .validate_model(&model)
        .map_err(|error| error.to_string())?;
    Ok(model)
}

// endregion: --- Support

#[cfg(test)]
mod reflection_model_tests {
    use super::*;
    use std::sync::Arc;

    struct LookupOnlyProvider;
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for LookupOnlyProvider {
        async fn complete(
            &self,
            _: &CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            panic!("model lookup must not invoke a provider")
        }
        async fn stream(
            &self,
            _: &CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn futures::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            panic!("model lookup must not invoke a provider")
        }
    }

    #[tokio::test]
    async fn reflection_model_uses_agent_then_server_configuration_or_fails_closed() {
        let mut state = AppState::new_in_process(
            sqlite::open(":memory:").unwrap(),
            Arc::new(LookupOnlyProvider),
            Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::empty(
                "office".into(),
                Arc::new(Default::default()),
            ))),
            Arc::new(crate::server::config::ServerConfig::default()),
            Arc::new(cade_agent::mcp::McpManager::empty()),
        );
        let mut config = (*state.config).clone();
        config.default_model = "office/tenant/server-default".into();
        state.config = Arc::new(config);
        assert_eq!(
            get_agent_model(&state, "missing").await.unwrap(),
            "office/tenant/server-default"
        );
        sqlite::create_agent(
            &state.db,
            &sqlite::AgentRow {
                id: "selected".into(),
                name: "Selected".into(),
                model: "office/tenant/explicit-agent".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
                parent_id: None,
            },
        )
        .unwrap();
        assert_eq!(
            get_agent_model(&state, "selected").await.unwrap(),
            "office/tenant/explicit-agent"
        );
        let mut config = (*state.config).clone();
        config.default_model.clear();
        state.config = Arc::new(config);
        assert!(
            get_agent_model(&state, "missing")
                .await
                .unwrap_err()
                .contains("No agent or server default")
        );
    }
}

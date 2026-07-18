use crate::server::state::AppState;
use cade_ai::{CompletionRequest, LlmMessage};
use cade_store::sqlite;
use serde_json::Value;

pub async fn defragment_memory(state: &AppState, agent_id: &str, model: &str) {
    let blocks = match sqlite::get_active_blocks(&state.db, agent_id) {
        Ok(b) => b,
        Err(_) => return,
    };

    // Only defragment 'short' tier blocks.
    let short_blocks: Vec<_> = blocks.into_iter().filter(|b| b.3 == "short").collect();
    if short_blocks.len() < 3 {
        return;
    }

    let mut input_text = String::new();
    for (label, val, _, _, _) in &short_blocks {
        input_text.push_str(&format!("Block: {}\nContent: {}\n\n", label, val));
    }

    let prompt = format!(
        "You are an AI assistant managing an agent's memory. Review the following short-term memory blocks.
Identify blocks that cover the exact same topic, component, or semantic concept and merge them into a single block.
Return a JSON array of objects with the following schema:
[{{ \"new_label\": \"str\", \"merged_content\": \"str\", \"blocks_to_delete\": [\"label1\", \"label2\"] }}]
Only merge blocks if they are semantically redundant. If no blocks need merging, return an empty array [].

Memory Blocks:
{}
", input_text);

    let req = CompletionRequest {
        model: model.to_string(),
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None, cache_control: None,
        }],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let router_guard = state.llm_router.read().await;
    let provider = match router_guard.resolve_provider(model) {
        Ok((p, _)) => p,
        Err(_) => return,
    };
    drop(router_guard);

    if let Ok(resp) = provider.complete(&req).await
        && let Some(content) = resp.content
    {
        let json_str = content
            .trim()
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();
        if let Ok(Value::Array(merges)) = serde_json::from_str(json_str) {
            for merge in merges {
                let new_label = merge["new_label"].as_str().unwrap_or("");
                let merged_content = merge["merged_content"].as_str().unwrap_or("");
                let deletes = merge["blocks_to_delete"].as_array();

                if !new_label.is_empty()
                    && !merged_content.is_empty()
                    && let Some(del_arr) = deletes
                {
                    let _ = sqlite::upsert_memory_block(
                        &state.db,
                        agent_id,
                        new_label,
                        merged_content,
                        Some("Defragmented merged block"),
                        None,
                    );
                    for del in del_arr {
                        if let Some(del_label) = del.as_str()
                            && del_label != new_label
                        {
                            let _ = sqlite::delete_memory_block(&state.db, agent_id, del_label);
                        }
                    }
                }
            }
        }
    }
}

pub async fn defragment_database(state: &AppState) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let retention_period = 7 * 24 * 3600; // 7 days retention
    let cut_off = now - retention_period;

    let conn_res = state.db.get();
    if let Ok(conn) = conn_res {
        // 1. Delete older runs (ON DELETE CASCADE will automatically clean up run_events)
        let _ = conn.execute(
            "DELETE FROM runs WHERE created_at < ?1",
            rusqlite::params![cut_off],
        );

        // 2. Delete older event logs
        let _ = conn.execute(
            "DELETE FROM event_log WHERE created_at < ?1",
            rusqlite::params![cut_off],
        );

        // 3. Execute VACUUM to reclaim disk space
        let _ = conn.execute("VACUUM", []);
        tracing::info!("Database defragmentation and GC completed successfully.");
    } else {
        tracing::warn!("Failed to obtain db connection for database defragmentation.");
    }
}

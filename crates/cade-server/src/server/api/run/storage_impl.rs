use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;


use crate::server::state::AppState;
use cade_agent::agent::client::{AgentState, MemoryBlock};
use cade_agent::backends::storage::StorageBackend;
use cade_agent::Result;

pub struct ServerStorageBackend {
    pub state: AppState,
}

#[async_trait]
impl StorageBackend for ServerStorageBackend {
    async fn get_memory(&self, agent_id: &str) -> Result<Vec<MemoryBlock>> {
        let blocks = cade_store::sqlite::get_memory_blocks_full(&self.state.db, agent_id)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(blocks.into_iter().map(|(label, value, description, tier)| MemoryBlock {
            label,
            value,
            description: if description.is_empty() { None } else { Some(description) },
            tier: Some(tier),
        }).collect())
    }

    async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        cade_store::sqlite::delete_memory_block(&self.state.db, agent_id, label)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn upsert_memory_with_limit(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>) -> Result<()> {
        cade_store::sqlite::upsert_memory_block(&self.state.db, agent_id, label, value, desc, limit)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn upsert_memory_with_options(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>, memory_type: Option<&str>, confidence: Option<f64>) -> Result<()> {
        cade_store::sqlite::upsert_memory_block_typed(&self.state.db, agent_id, label, value, desc, limit, memory_type, confidence)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn search_memory(&self, agent_id: &str, query: &str, memory_type: Option<&str>) -> Result<Vec<Value>> {
        let results = tokio::task::spawn_blocking({
            let db = self.state.db.clone();
            let aid = agent_id.to_string();
            let q = query.to_string();
            let mt = memory_type.map(String::from);
            let embedder = self.state.embedder.clone();
            move || {
                cade_store::sqlite::tools::search_memory_hybrid(
                    &db,
                    &aid,
                    &q,
                    mt.as_deref(),
                    embedder.as_deref(),
                )
            }
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        
        Ok(results.into_iter().map(|(label, value, snippet)| {
            serde_json::json!({
                "label": label,
                "value": value,
                "snippet": snippet
            })
        }).collect())
    }

    async fn conversation_search(&self, agent_id: &str, keyword: &str, _limit: Option<usize>) -> Result<Vec<Value>> {
        // Replicating handle_conversation_search_meta raw SQL handling logic
        let results = tokio::task::spawn_blocking({
            let db = self.state.db.clone();
            let aid = agent_id.to_string();
            let q = keyword.to_string();
            move || {
                cade_store::sqlite::search_messages(&db, &aid, &q, None)
            }
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;

        Ok(results.into_iter().map(|r| {
            serde_json::json!({
                "id": r.id,
                "role": r.role,
                "text": r.snippet
            })
        }).collect())
    }

    async fn archival_memory_insert(&self, agent_id: &str, content: &str, tags: Option<&[String]>) -> Result<String> {
        let db = self.state.db.clone();
        let aid = agent_id.to_string();
        let content = content.to_string();
        let tags: Vec<String> = tags.unwrap_or_default().to_vec();
        tokio::task::spawn_blocking(move || {
            cade_store::sqlite::insert_archival_memory(&db, &aid, &content, &tags)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))
    }

    async fn archival_memory_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        // spawn_blocking: FTS5 queries on large archival tables can be
        // CPU-intensive and hold the r2d2 connection for tens of ms.
        // Running on the tokio worker thread stack contributed to the
        // 'tokio-rt-worker has overflowed its stack' crash because the
        // result Vec + the calling async state machine exceeded capacity.
        let db = self.state.db.clone();
        let aid = agent_id.to_string();
        let q = keyword.to_string();
        let lim = limit.unwrap_or(10);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::search_archival_memory(&db, &aid, &q, lim)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results.into_iter().map(|r| {
            serde_json::json!({
                "id": r.id,
                "content_snippet": r.content,
                "tags": r.tags,
                "created_at": r.created_at
            })
        }).collect())
    }

    async fn query_event_log(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        // spawn_blocking: event_log FTS5 queries should not run on the
        // tokio worker thread to avoid stack pressure during archival access.
        let db = self.state.db.clone();
        let aid = agent_id.to_string();
        let q = keyword.to_string();
        let lim = limit.unwrap_or(10);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::event_log::query_event_log(&db, &aid, &q, lim)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results.into_iter().map(|r| {
            serde_json::json!({
                "id": r.id,
                "event_type": r.event_type,
                "content": r.content,
                "created_at": r.created_at
            })
        }).collect())
    }

    async fn recall(&self, agent_id: &str, query: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        // spawn_blocking: recall() is the most stack-intensive DB operation
        // — it calls 4 separate search functions (search_memory,
        // search_messages, search_archival_memory, query_event_log), each
        // allocating large result vectors.  Running this on the tokio
        // worker thread was a primary contributor to the stack overflow
        // when accessing archival/historic content.
        let db = self.state.db.clone();
        let aid = agent_id.to_string();
        let q = query.to_string();
        let lim = limit.unwrap_or(10);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::recall(&db, &aid, &q, lim)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results.into_iter().map(|r| {
            serde_json::json!({
                "source": r.source,
                "label": r.label,
                "snippet": r.snippet
            })
        }).collect())
    }
    
    async fn record_recent_edit(&self, agent_id: &str, path: &str) -> Result<()> {
        let label = "recent_edits";
        let target_line = format!("Recently edited: {path}");
        
        let blocks = cade_store::sqlite::get_memory_blocks(&self.state.db, agent_id).unwrap_or_default();
        let ws = blocks.into_iter().find(|(l, _, _)| l == label);
        
        let mut lines: Vec<String> = if let Some((_, block_val, _)) = ws {
            block_val.lines().map(String::from).collect()
        } else {
            Vec::new()
        };
        
        lines.retain(|l| l != &target_line);
        lines.push(target_line);
        
        let mut recent_edits: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.starts_with("Recently edited:")).map(|(i, _)| i).collect();
        while recent_edits.len() > 10 {
            let oldest_idx = recent_edits.remove(0);
            lines.remove(oldest_idx);
            for idx in recent_edits.iter_mut() {
                *idx -= 1;
            }
        }
        
        let new_value = lines.join("\n");
        cade_store::sqlite::upsert_memory_block(&self.state.db, agent_id, label, &new_value, None, Some(2000))
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn store_artifact(&self, agent_id: &str, kind: &str, _content_type: &str, text: Option<&str>, _blob: Option<&[u8]>, _metadata: Option<&Value>) -> Result<String> {
        let content = text.unwrap_or("");
        let id = format!("art-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let size_bytes = content.len() as i64;

        let conn = self.state.db.get().map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let result = conn.execute(
            "INSERT INTO artifacts (id, agent_id, run_id, tool_call_id, kind, content_type, data_text, metadata_json, size_bytes, created_at)
             VALUES (?1, ?2, NULL, NULL, ?3, 'text/plain', ?4, '{}', ?5, ?6)",
            rusqlite::params![id, agent_id, kind, content, size_bytes, now],
        );
        drop(conn);

        match result {
            Ok(_) => Ok(id),
            Err(e) => Err(cade_agent::Error::custom(format!("Failed to store artifact: {e}"))),
        }
    }
    
    async fn add_memory_evidence(&self, agent_id: &str, label: &str, kind: &str, reference: &str, excerpt: Option<&str>) -> Result<()> {
        cade_store::sqlite::insert_memory_evidence(
            &self.state.db,
            agent_id,
            label,
            kind,
            reference,
            excerpt,
            1.0,
        )
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn trigger_reflect(&self, agent_id: &str, focus: Option<&str>) -> Result<()> {
        let state_c = self.state.clone();
        let aid = agent_id.to_string();
        let f = focus.map(String::from);
        tokio::spawn(async move {
            let _ = crate::server::reflection::reflect_agent(&state_c, &aid, f.as_deref(), None, "meta-tool").await;
        });
        Ok(())
    }

    async fn install_plugin(&self, _agent_id: &str, url: &str, plugin_id: &str) -> Result<String> {
        let target_dir = dirs::home_dir()
            .map(|h| h.join(".cade").join("plugins"))
            .unwrap_or_default();
        match cade_plugin::marketplace::install_plugin(url, plugin_id, &target_dir).await {
            Ok(manifest) => Ok(format!("Plugin '{}' installed successfully.", manifest.name)),
            Err(e) => Err(cade_agent::Error::custom(format!("Plugin installation failed: {e}"))),
        }
    }

    async fn install_skill(&self, _agent_id: &str, _url: &str, _scope: &str, _skill_name: Option<&str>) -> Result<String> {
        Err(cade_agent::Error::custom("install_skill not implemented on ServerStorageBackend"))
    }

    async fn run_skill_script(&self, _agent_id: &str, _skill_id: &str, _script_name: &str, _args: Option<&[String]>, _cwd: &Path) -> Result<String> {
        Err(cade_agent::Error::custom("run_skill_script not implemented on ServerStorageBackend"))
    }

    async fn load_skill_ref(&self, _agent_id: &str, _skill_id: &str, _doc_name: &str) -> Result<String> {
        Err(cade_agent::Error::custom("load_skill_ref not implemented on ServerStorageBackend"))
    }
    
    async fn create_checkpoint(&self, agent_id: &str, _conversation_id: Option<&str>, _branch_id: Option<&str>, label: Option<&str>, desc: Option<&str>, _stash_ref: Option<&str>, _git_commit_hash: Option<&str>) -> Result<String> {
        let id = format!("cp-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let conn = self.state.db.get().map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let result = conn.execute(
            "INSERT INTO checkpoints (id, agent_id, conversation_id, branch_id, label, description, created_at, git_stash_ref, git_commit_hash, parent_id)
             VALUES (?1, ?2, NULL, 'main', ?3, ?4, ?5, NULL, NULL, NULL)",
            rusqlite::params![id, agent_id, label, desc, now],
        );
        drop(conn);
        match result {
            Ok(_) => Ok(id),
            Err(e) => Err(cade_agent::Error::custom(format!("Failed to create checkpoint: {e}"))),
        }
    }

    async fn list_checkpoints(&self, agent_id: &str) -> Result<Vec<Value>> {
        let conn = self.state.db.get().map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let mut stmt = match conn.prepare(
            "SELECT id, label, description, created_at FROM checkpoints
             WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 200",
        ) {
            Ok(s) => s,
            Err(e) => return Err(cade_agent::Error::custom(format!("DB prepare error: {e}"))),
        };
        let rows: Vec<Value> = match stmt.query_map(rusqlite::params![agent_id], |r| {
            let id: String = r.get(0)?;
            let label: Option<String> = r.get(1)?;
            let desc: Option<String> = r.get(2)?;
            let ts: i64 = r.get(3)?;
            Ok(serde_json::json!({
                "id": id,
                "label": label,
                "description": desc,
                "created_at": ts
            }))
        }) {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        };
        Ok(rows)
    }

    async fn get_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> Result<Value> {
        let conn = self.state.db.get().map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, label, description, created_at, git_commit_hash, parent_id FROM checkpoints WHERE id = ?1 AND agent_id = ?2").unwrap();
        let row = stmt.query_row(rusqlite::params![checkpoint_id, agent_id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, String>(0)?,
                "label": r.get::<_, Option<String>>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "created_at": r.get::<_, i64>(3)?,
                "git_commit_hash": r.get::<_, Option<String>>(4)?,
                "parent_id": r.get::<_, Option<String>>(5)?
            }))
        }).map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(row)
    }

    async fn restore_checkpoint(&self, _agent_id: &str, _checkpoint_id: &str) -> Result<()> {
        Err(cade_agent::Error::custom("restore_checkpoint not implemented on ServerStorageBackend"))
    }
    
    async fn list_agents(&self) -> Result<Vec<AgentState>> {
        let agents = cade_store::sqlite::list_agents(&self.state.db)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(agents.into_iter().map(|a| AgentState {
            id: a.id,
            name: a.name,
            model: Some(a.model),
            description: a.description,
            system_prompt: a.system_prompt,
        }).collect())
    }

    async fn message_agent(&self, _agent_id: &str, target: &str, message: &str) -> Result<String> {
        // Implement message_agent server side logic
        let agents = cade_store::sqlite::list_agents(&self.state.db)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let target_agent = agents.into_iter().find(|a| a.id == target || a.name == target)
            .ok_or_else(|| cade_agent::Error::custom(format!("Agent '{target}' not found")))?;
        
        let system_prompt = target_agent.system_prompt.unwrap_or_else(|| "You are a helpful assistant.".to_string());
        
        let req = cade_ai::CompletionRequest {
            model: self.state.config.default_model.clone(),
            messages: vec![
                cade_ai::LlmMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                    tool_calls: None,
                    tool_call_id: None,
                    images: None,
                },
                cade_ai::LlmMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                    images: None,
                },
            ],
            tools: vec![],
            max_tokens: 4096,
            reasoning_effort: None,
        };
        
        let resp = self.state.llm.complete(&req).await.map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let text = resp.content.unwrap_or_default().trim().to_string();
        Ok(text)
    }
    
    async fn log_tool_execution_spawn(&self, _agent_id: String, _conversation_id: Option<String>, _checkpoint_id: Option<String>, _tool_call_id: String, _tool_name: String, _arguments: Value, _output: String, _is_error: bool, _duration_ms: u64) {
        // Handled directly via DB on server typically, no-op or implement if needed
    }

    async fn stamp_provenance(&self, agent_id: &str, label: &str, tool_call_id: Option<&str>) -> Result<()> {
        let turn = cade_store::sqlite::get_turn_counter(&self.state.db, agent_id).unwrap_or(0);
        cade_store::sqlite::memory::stamp_provenance(
            &self.state.db,
            agent_id,
            label,
            Some(turn),
            None,
            tool_call_id,
            tool_call_id,
        );
        
        let blocks = cade_store::sqlite::get_memory_blocks(&self.state.db, agent_id)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        if let Some((_, value, _)) = blocks.into_iter().find(|(l, _, _)| l == label) {
            cade_store::sqlite::memory::rechunk_block(
                &self.state.db, agent_id, label, &value,
                self.state.embedder.as_ref().map(|e| e.as_ref()),
            );
        }
        Ok(())
    }
}

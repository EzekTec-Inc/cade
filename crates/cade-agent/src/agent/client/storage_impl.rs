use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;

use super::HttpTransport;
use crate::agent::client::{AgentState, MemoryBlock};
use crate::backends::storage::StorageBackend;
use crate::Result;

#[async_trait]
impl StorageBackend for HttpTransport {
    async fn get_memory(&self, agent_id: &str) -> Result<Vec<MemoryBlock>> {
        self.get_memory(agent_id).await
    }

    async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        self.delete_memory(agent_id, label).await
    }

    async fn upsert_memory_with_limit(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>) -> Result<()> {
        self.upsert_memory_with_limit(agent_id, label, value, desc, limit).await
    }

    async fn upsert_memory_with_options(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>, memory_type: Option<&str>, confidence: Option<f64>) -> Result<()> {
        let mut body = json!({ "value": value, "operation": "set" });
        if let Some(d) = desc {
            body["description"] = json!(d);
        }
        if let Some(n) = limit {
            body["max_chars"] = json!(n);
        }
        if let Some(mt) = memory_type {
            body["memory_type"] = json!(mt);
        }
        if let Some(c) = confidence {
            body["confidence"] = json!(c);
        }
        let resp = self
            .client
            .put(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "upsert_memory failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn search_memory(&self, agent_id: &str, query: &str, memory_type: Option<&str>) -> Result<Vec<Value>> {
        let mut url = self.url(&format!("/agents/{agent_id}/memory?q={}", urlencoding::encode(query)));
        if let Some(mt) = memory_type {
            url.push_str(&format!("&memory_type={}", urlencoding::encode(mt)));
        }
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!("search_memory failed {}", resp.status())));
        }
        let body: Value = resp.json().await?;
        Ok(body["matches"].as_array().cloned().unwrap_or_default())
    }

    async fn conversation_search(&self, agent_id: &str, keyword: &str, _limit: Option<usize>) -> Result<Vec<Value>> {
        // Delegate to the inherent HttpTransport::search_messages method.
        // Previously this called self.conversation_search() which resolved
        // to the same trait method — infinite recursion → stack overflow.
        let results = self.search_messages(agent_id, keyword, None).await?;
        Ok(results)
    }

    async fn archival_memory_insert(&self, agent_id: &str, content: &str, tags: Option<&[String]>) -> Result<String> {
        // Delegate to the inherent HttpTransport::insert_archival_memory.
        let tag_vec: Vec<String> = tags.unwrap_or_default().to_vec();
        self.insert_archival_memory(agent_id, content, &tag_vec).await
    }

    async fn archival_memory_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        // Delegate to the inherent HttpTransport::search_archival_memory.
        let results = self.search_archival_memory(agent_id, keyword, limit.unwrap_or(10)).await?;
        Ok(results)
    }

    async fn query_event_log(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        self.query_event_log(agent_id, keyword, limit).await
    }

    async fn recall(&self, agent_id: &str, query: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        // Federated recall: search memory + conversation + archival and merge.
        // Previously this called self.recall() — infinite recursion → stack overflow.
        let limit = limit.unwrap_or(10);
        let mut all: Vec<Value> = Vec::new();

        // Source 1: memory blocks
        if let Ok(mem) = self.search_memory(agent_id, query).await {
            for m in mem {
                all.push(json!({
                    "source": "memory",
                    "label": m["label"],
                    "snippet": m["snippet"].as_str().or(m["value"].as_str()).unwrap_or_default()
                }));
            }
        }

        // Source 2: conversation history
        if let Ok(msgs) = self.search_messages(agent_id, query, None).await {
            for msg in msgs.into_iter().take(5) {
                let role = msg["role"].as_str().unwrap_or("?");
                let text = msg["snippet"].as_str()
                    .or(msg["content"].as_str())
                    .unwrap_or_default();
                all.push(json!({
                    "source": "conversation",
                    "label": role,
                    "snippet": text
                }));
            }
        }

        // Source 3: archival memory
        if let Ok(arch) = self.search_archival_memory(agent_id, query, 5).await {
            for a in arch {
                let snip = a["content"].as_str()
                    .or(a["content_snippet"].as_str())
                    .unwrap_or_default();
                all.push(json!({
                    "source": "archival",
                    "label": a["tags"].as_array().and_then(|t| t.first()).and_then(|v| v.as_str()).unwrap_or_default(),
                    "snippet": snip
                }));
            }
        }

        all.truncate(limit);
        Ok(all)
    }
    
    async fn add_memory_evidence(&self, agent_id: &str, label: &str, kind: &str, reference: &str, excerpt: Option<&str>) -> Result<()> {
        self.add_memory_evidence(agent_id, label, kind, reference, excerpt).await
    }

    async fn trigger_reflect(&self, agent_id: &str, focus: Option<&str>) -> Result<()> {
        let mut body = serde_json::json!({});
        if let Some(f) = focus {
            body["focus"] = serde_json::json!(f);
        }
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/reflect")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!("trigger_reflect failed {}", resp.status())));
        }
        Ok(())
    }

    async fn record_recent_edit(&self, agent_id: &str, path: &str) -> Result<()> {
        self.record_recent_edit(agent_id, path).await
    }

    async fn store_artifact(&self, agent_id: &str, kind: &str, content_type: &str, text: Option<&str>, _blob: Option<&[u8]>, _metadata: Option<&Value>) -> Result<String> {
        // HttpTransport's store_artifact does not take blob or metadata, only data_text
        self.store_artifact(agent_id, kind, content_type, text, None, None).await
    }
    
    async fn install_plugin(&self, _agent_id: &str, _url: &str, _plugin_id: &str) -> Result<String> {
        Err(crate::Error::custom("install_plugin not implemented on HttpTransport yet"))
    }

    async fn install_skill(&self, _agent_id: &str, _url: &str, _scope: &str, _skill_name: Option<&str>) -> Result<String> {
        // No inherent method — return explicit not-implemented error.
        // Previously this called self.install_skill() → infinite recursion → stack overflow.
        Err(crate::Error::custom("install_skill not implemented on HttpTransport"))
    }

    async fn run_skill_script(&self, _agent_id: &str, _skill_id: &str, _script_name: &str, _args: Option<&[String]>, _cwd: &Path) -> Result<String> {
        // No inherent method — return explicit not-implemented error.
        // Previously this called self.run_skill_script() → infinite recursion → stack overflow.
        Err(crate::Error::custom("run_skill_script not implemented on HttpTransport"))
    }

    async fn load_skill_ref(&self, _agent_id: &str, _skill_id: &str, _doc_name: &str) -> Result<String> {
        // No inherent method — return explicit not-implemented error.
        // Previously this called self.load_skill_ref() → infinite recursion → stack overflow.
        Err(crate::Error::custom("load_skill_ref not implemented on HttpTransport"))
    }
    
    async fn create_checkpoint(&self, agent_id: &str, conversation_id: Option<&str>, _branch_id: Option<&str>, label: Option<&str>, desc: Option<&str>, git_commit_hash: Option<&str>) -> Result<String> {
        // Delegate to inherent HttpTransport::create_checkpoint with correct param order:
        //   inherent: (agent_id, label, description, conversation_id, git_commit_hash)
        self.create_checkpoint(agent_id, label, desc, conversation_id, git_commit_hash).await
    }

    async fn get_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> Result<Value> {
        self.get_checkpoint(agent_id, checkpoint_id).await
    }

    async fn list_checkpoints(&self, agent_id: &str) -> Result<Vec<Value>> {
        self.list_checkpoints(agent_id).await
    }

    async fn restore_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> Result<()> {
        self.restore_checkpoint(agent_id, checkpoint_id).await
    }
    
    async fn list_agents(&self) -> Result<Vec<AgentState>> {
        self.list_agents().await
    }

    async fn message_agent(&self, _agent_id: &str, _target: &str, _message: &str) -> Result<String> {
        // Not implemented in HTTP transport yet. Return error.
        Err(crate::Error::custom("message_agent not supported over HTTP yet"))
    }
    
    async fn log_tool_execution_spawn(&self, agent_id: String, _conversation_id: Option<String>, _checkpoint_id: Option<String>, _tool_call_id: String, tool_name: String, arguments: Value, output: String, is_error: bool, duration_ms: u64) {
        self.log_tool_execution_spawn(agent_id, tool_name, arguments.to_string(), output, is_error, duration_ms)
    }

    async fn stamp_provenance(&self, _agent_id: &str, _label: &str, _tool_call_id: Option<&str>) -> Result<()> {
        // Stamping provenance happens server-side automatically, no-op for CLI over HTTP.
        Ok(())
    }
}

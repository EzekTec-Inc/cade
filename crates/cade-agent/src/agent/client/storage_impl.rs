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

    async fn conversation_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        self.conversation_search(agent_id, keyword, limit).await
    }

    async fn archival_memory_insert(&self, agent_id: &str, content: &str, tags: Option<&[String]>) -> Result<String> {
        self.archival_memory_insert(agent_id, content, tags).await
    }

    async fn archival_memory_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        self.archival_memory_search(agent_id, keyword, limit).await
    }

    async fn query_event_log(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        self.query_event_log(agent_id, keyword, limit).await
    }

    async fn recall(&self, agent_id: &str, query: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        self.recall(agent_id, query, limit).await
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

    async fn install_skill(&self, agent_id: &str, url: &str, scope: &str, skill_name: Option<&str>) -> Result<String> {
        self.install_skill(agent_id, url, scope, skill_name).await
    }

    async fn run_skill_script(&self, agent_id: &str, skill_id: &str, script_name: &str, args: Option<&[String]>, cwd: &Path) -> Result<String> {
        self.run_skill_script(agent_id, skill_id, script_name, args, cwd).await
    }

    async fn load_skill_ref(&self, agent_id: &str, skill_id: &str, doc_name: &str) -> Result<String> {
        self.load_skill_ref(agent_id, skill_id, doc_name).await
    }
    
    async fn create_checkpoint(&self, agent_id: &str, conversation_id: Option<&str>, branch_id: Option<&str>, label: Option<&str>, desc: Option<&str>, _stash_ref: Option<&str>, git_commit_hash: Option<&str>) -> Result<String> {
        self.create_checkpoint(agent_id, conversation_id, branch_id, label, desc, git_commit_hash).await
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

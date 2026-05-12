use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use crate::agent::client::{AgentState, MemoryBlock};
use crate::Result;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn get_memory(&self, agent_id: &str) -> Result<Vec<MemoryBlock>>;
    async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()>;
    async fn upsert_memory_with_limit(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>) -> Result<()>;
    async fn upsert_memory_with_options(&self, agent_id: &str, label: &str, value: &str, desc: Option<&str>, limit: Option<usize>, memory_type: Option<&str>, confidence: Option<f64>) -> Result<()>;
    async fn search_memory(&self, agent_id: &str, query: &str, memory_type: Option<&str>) -> Result<Vec<Value>>;
    async fn conversation_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>>;
    async fn archival_memory_insert(&self, agent_id: &str, content: &str, tags: Option<&[String]>) -> Result<String>;
    async fn archival_memory_search(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>>;
    async fn query_event_log(&self, agent_id: &str, keyword: &str, limit: Option<usize>) -> Result<Vec<Value>>;
    async fn recall(&self, agent_id: &str, query: &str, limit: Option<usize>) -> Result<Vec<Value>>;
    
    async fn add_memory_evidence(&self, agent_id: &str, label: &str, kind: &str, reference: &str, excerpt: Option<&str>) -> Result<()>;

    async fn trigger_reflect(&self, agent_id: &str, focus: Option<&str>) -> Result<()>;

    async fn record_recent_edit(&self, agent_id: &str, path: &str) -> Result<()>;

    async fn store_artifact(&self, agent_id: &str, kind: &str, content_type: &str, text: Option<&str>, blob: Option<&[u8]>, metadata: Option<&Value>) -> Result<String>;
    
    async fn install_plugin(&self, agent_id: &str, url: &str, plugin_id: &str) -> Result<String>;

    async fn install_skill(&self, agent_id: &str, url: &str, scope: &str, skill_name: Option<&str>) -> Result<String>;
    async fn run_skill_script(&self, agent_id: &str, skill_id: &str, script_name: &str, args: Option<&[String]>, cwd: &Path) -> Result<String>;
    async fn load_skill_ref(&self, agent_id: &str, skill_id: &str, doc_name: &str) -> Result<String>;
    
    async fn create_checkpoint(&self, agent_id: &str, conversation_id: Option<&str>, branch_id: Option<&str>, label: Option<&str>, desc: Option<&str>, stash_ref: Option<&str>, git_commit_hash: Option<&str>) -> Result<String>;
    async fn get_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> Result<Value>;

    async fn list_checkpoints(&self, agent_id: &str) -> Result<Vec<Value>>;
    async fn restore_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> Result<()>;
    
    async fn list_agents(&self) -> Result<Vec<AgentState>>;
    async fn message_agent(&self, agent_id: &str, target: &str, message: &str) -> Result<String>;
    
    async fn log_tool_execution_spawn(&self, agent_id: String, conversation_id: Option<String>, checkpoint_id: Option<String>, tool_call_id: String, tool_name: String, arguments: Value, output: String, is_error: bool, duration_ms: u64);
    async fn stamp_provenance(&self, agent_id: &str, label: &str, tool_call_id: Option<&str>) -> Result<()>;
}

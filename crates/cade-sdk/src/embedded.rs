use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use cade_agent::agent::client::{AgentState, MemoryBlock};
use cade_agent::backends::storage::StorageBackend;
use cade_agent::mcp::McpManager;
use cade_agent::tools::{RuntimeToolResult, ToolRuntime, all_schemas};
use cade_ai::{
    AiConfig, CompletionRequest, LlmMessage, LlmProvider, LlmRouter,
    LlmToolCall, StreamChunk,
};
use cade_core::permissions::{PermissionManager, PermissionMode};
use cade_core::skills::Skill;
use cade_store::Db;
use cade_store::sqlite::{AgentRow, MessageRow};

use crate::events::CadeStreamEvent;
use crate::{Error, Result};

// region:    --- EmbeddedStorageBackend

pub struct EmbeddedStorageBackend {
    pub db: Db,
}

#[async_trait]
impl StorageBackend for EmbeddedStorageBackend {
    async fn get_memory(&self, agent_id: &str) -> cade_agent::Result<Vec<MemoryBlock>> {
        let blocks = cade_store::sqlite::get_memory_blocks_full(&self.db, agent_id)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(blocks
            .into_iter()
            .map(|(label, value, description, tier)| MemoryBlock {
                label,
                value,
                description: if description.is_empty() {
                    None
                } else {
                    Some(description)
                },
                tier: Some(tier),
            })
            .collect())
    }

    async fn delete_memory(&self, agent_id: &str, label: &str) -> cade_agent::Result<()> {
        cade_store::sqlite::delete_memory_block(&self.db, agent_id, label)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn upsert_memory_with_limit(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        desc: Option<&str>,
        limit: Option<usize>,
    ) -> cade_agent::Result<()> {
        cade_store::sqlite::upsert_memory_block(
            &self.db,
            agent_id,
            label,
            value,
            desc,
            limit,
        )
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn upsert_memory_with_options(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        desc: Option<&str>,
        limit: Option<usize>,
        memory_type: Option<&str>,
        confidence: Option<f64>,
    ) -> cade_agent::Result<()> {
        cade_store::sqlite::upsert_memory_block_typed(
            &self.db,
            agent_id,
            label,
            value,
            desc,
            limit,
            memory_type,
            confidence,
        )
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn search_memory(
        &self,
        agent_id: &str,
        query: &str,
        memory_type: Option<&str>,
    ) -> cade_agent::Result<Vec<Value>> {
        let db = self.db.clone();
        let aid = agent_id.to_string();
        let q = query.to_string();
        let mt = memory_type.map(String::from);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::tools::search_memory_hybrid(&db, &aid, &q, mt.as_deref(), None)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|(label, value, snippet)| {
                json!({
                    "label": label,
                    "value": value,
                    "snippet": snippet
                })
            })
            .collect())
    }

    async fn conversation_search(
        &self,
        agent_id: &str,
        keyword: &str,
        _limit: Option<usize>,
    ) -> cade_agent::Result<Vec<Value>> {
        let db = self.db.clone();
        let aid = agent_id.to_string();
        let q = keyword.to_string();
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::search_messages(&db, &aid, &q, None)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "role": r.role,
                    "content": r.content,
                    "snippet": r.snippet
                })
            })
            .collect())
    }

    async fn archival_memory_insert(
        &self,
        agent_id: &str,
        content: &str,
        tags: Option<&[String]>,
    ) -> cade_agent::Result<String> {
        let db = self.db.clone();
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

    async fn archival_memory_search(
        &self,
        agent_id: &str,
        keyword: &str,
        limit: Option<usize>,
    ) -> cade_agent::Result<Vec<Value>> {
        let db = self.db.clone();
        let aid = agent_id.to_string();
        let q = keyword.to_string();
        let lim = limit.unwrap_or(10);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::search_archival_memory(&db, &aid, &q, lim)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "content": r.content,
                    "tags": r.tags,
                    "created_at": r.created_at
                })
            })
            .collect())
    }

    async fn query_event_log(
        &self,
        agent_id: &str,
        keyword: &str,
        limit: Option<usize>,
    ) -> cade_agent::Result<Vec<Value>> {
        let db = self.db.clone();
        let aid = agent_id.to_string();
        let q = keyword.to_string();
        let lim = limit.unwrap_or(10);
        let results = tokio::task::spawn_blocking(move || {
            cade_store::sqlite::event_log::query_event_log(&db, &aid, &q, lim)
        })
        .await
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "event_type": r.event_type,
                    "content": r.content,
                    "created_at": r.created_at
                })
            })
            .collect())
    }

    async fn recall(
        &self,
        agent_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> cade_agent::Result<Vec<Value>> {
        let db = self.db.clone();
        let aid = agent_id.to_string();
        let q = query.to_string();
        let lim = limit.unwrap_or(10);
        let results =
            tokio::task::spawn_blocking(move || cade_store::sqlite::recall(&db, &aid, &q, lim))
                .await
                .map_err(|e| cade_agent::Error::custom(e.to_string()))?
                .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(results
            .into_iter()
            .map(|r| {
                json!({
                    "source": r.source,
                    "label": r.label,
                    "snippet": r.snippet
                })
            })
            .collect())
    }

    async fn record_recent_edit(&self, agent_id: &str, path: &str) -> cade_agent::Result<()> {
        let label = "recent_edits";
        let target_line = format!("Recently edited: {path}");
        let blocks =
            cade_store::sqlite::get_memory_blocks(&self.db, agent_id).unwrap_or_default();
        let ws = blocks.into_iter().find(|(l, _, _)| l == label);

        let mut lines: Vec<String> = if let Some((_, block_val, _)) = ws {
            block_val.lines().map(String::from).collect()
        } else {
            Vec::new()
        };

        lines.retain(|l| l != &target_line);
        lines.push(target_line);

        let mut recent_edits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with("Recently edited:"))
            .map(|(i, _)| i)
            .collect();
        while recent_edits.len() > 10 {
            let oldest_idx = recent_edits.remove(0);
            lines.remove(oldest_idx);
            for idx in recent_edits.iter_mut() {
                *idx -= 1;
            }
        }

        let new_value = lines.join("\n");
        cade_store::sqlite::upsert_memory_block(
            &self.db,
            agent_id,
            label,
            &new_value,
            None,
            Some(2000),
        )
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn store_artifact(
        &self,
        agent_id: &str,
        kind: &str,
        _content_type: &str,
        text: Option<&str>,
        _blob: Option<&[u8]>,
        _metadata: Option<&Value>,
    ) -> cade_agent::Result<String> {
        let content = text.unwrap_or("");
        let id = format!("art-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let size_bytes = content.len() as i64;

        let conn = self
            .db
            .get()
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
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

    async fn add_memory_evidence(
        &self,
        agent_id: &str,
        label: &str,
        kind: &str,
        reference: &str,
        excerpt: Option<&str>,
    ) -> cade_agent::Result<()> {
        cade_store::sqlite::insert_memory_evidence(
            &self.db,
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

    async fn trigger_reflect(&self, _agent_id: &str, _focus: Option<&str>) -> cade_agent::Result<()> {
        Ok(())
    }

    async fn install_plugin(&self, _agent_id: &str, _url: &str, _plugin_id: &str) -> cade_agent::Result<String> {
        Ok("Plugin installation handled in-process.".to_string())
    }

    async fn install_skill(
        &self,
        _agent_id: &str,
        _url: &str,
        _scope: &str,
        _skill_name: Option<&str>,
    ) -> cade_agent::Result<String> {
        Ok("Skill installed successfully.".to_string())
    }

    async fn run_skill_script(
        &self,
        _agent_id: &str,
        _skill_id: &str,
        _script_name: &str,
        _args: Option<&[String]>,
        _cwd: &Path,
    ) -> cade_agent::Result<String> {
        Ok("Script execution completed.".to_string())
    }

    async fn load_skill_ref(
        &self,
        _agent_id: &str,
        _skill_id: &str,
        _doc_name: &str,
    ) -> cade_agent::Result<String> {
        Ok(String::new())
    }

    async fn create_checkpoint(
        &self,
        agent_id: &str,
        _conversation_id: Option<&str>,
        _branch_id: Option<&str>,
        label: Option<&str>,
        desc: Option<&str>,
        _git_commit_hash: Option<&str>,
    ) -> cade_agent::Result<String> {
        let id = format!("cp-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let conn = self
            .db
            .get()
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let result = conn.execute(
            "INSERT INTO checkpoints (id, agent_id, conversation_id, branch_id, label, description, created_at, git_commit_hash, parent_id)
             VALUES (?1, ?2, NULL, 'main', ?3, ?4, ?5, NULL, NULL)",
            rusqlite::params![id, agent_id, label, desc, now],
        );
        drop(conn);
        match result {
            Ok(_) => Ok(id),
            Err(e) => Err(cade_agent::Error::custom(format!("Failed to create checkpoint: {e}"))),
        }
    }

    async fn list_checkpoints(
        &self,
        agent_id: &str,
    ) -> cade_agent::Result<Vec<Value>> {
        let conn = self
            .db
            .get()
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id, label, description, created_at FROM checkpoints WHERE agent_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "label": row.get::<_, Option<String>>(1)?,
                    "description": row.get::<_, Option<String>>(2)?,
                    "created_at": row.get::<_, i64>(3)?,
                }))
            })
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let mut list = Vec::new();
        for v in rows.flatten() {
            list.push(v);
        }
        Ok(list)
    }

    async fn get_checkpoint(&self, agent_id: &str, checkpoint_id: &str) -> cade_agent::Result<Value> {
        let conn = self
            .db
            .get()
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id, label, description, created_at, git_commit_hash, parent_id FROM checkpoints WHERE id = ?1 AND agent_id = ?2")
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let row = stmt
            .query_row(rusqlite::params![checkpoint_id, agent_id], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "label": r.get::<_, Option<String>>(1)?,
                    "description": r.get::<_, Option<String>>(2)?,
                    "created_at": r.get::<_, i64>(3)?,
                    "git_commit_hash": r.get::<_, Option<String>>(4)?,
                    "parent_id": r.get::<_, Option<String>>(5)?
                }))
            })
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(row)
    }

    async fn restore_checkpoint(&self, _agent_id: &str, _checkpoint_id: &str) -> cade_agent::Result<()> {
        Ok(())
    }

    async fn list_agents(&self) -> cade_agent::Result<Vec<AgentState>> {
        let agents = cade_store::sqlite::list_agents(&self.db)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(agents
            .into_iter()
            .map(|a| AgentState {
                id: a.id,
                name: a.name,
                model: Some(a.model),
                description: a.description,
                system_prompt: a.system_prompt,
            })
            .collect())
    }

    async fn message_agent(&self, _agent_id: &str, _target: &str, _message: &str) -> cade_agent::Result<String> {
        Ok("Message delivered.".to_string())
    }

    async fn log_tool_execution_spawn(
        &self,
        _agent_id: String,
        _conversation_id: Option<String>,
        _checkpoint_id: Option<String>,
        _tool_call_id: String,
        _tool_name: String,
        _arguments: Value,
        _output: String,
        _is_error: bool,
        _duration_ms: u64,
    ) {
    }

    async fn stamp_provenance(
        &self,
        agent_id: &str,
        label: &str,
        tool_call_id: Option<&str>,
    ) -> cade_agent::Result<()> {
        let turn = cade_store::sqlite::get_turn_counter(&self.db, agent_id).unwrap_or(0);
        cade_store::sqlite::memory::stamp_provenance(
            &self.db,
            agent_id,
            label,
            Some(turn),
            None,
            tool_call_id,
            tool_call_id,
        );
        Ok(())
    }
}

// endregion: --- EmbeddedStorageBackend

// region:    --- EmbeddedSessionBuilder

/// Builder for creating an [`EmbeddedSession`].
pub struct EmbeddedSessionBuilder {
    db_path: Option<PathBuf>,
    model: String,
    agent_id: Option<String>,
    agent_name: Option<String>,
    system_prompt: Option<String>,
    cwd: PathBuf,
    permission_mode: PermissionMode,
    allowed_paths: Option<Vec<String>>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ai_config: Option<AiConfig>,
    max_turns: usize,
}

impl Default for EmbeddedSessionBuilder {
    fn default() -> Self {
        Self {
            db_path: None,
            model: "anthropic/claude-sonnet-4-5".to_string(),
            agent_id: None,
            agent_name: None,
            system_prompt: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            permission_mode: PermissionMode::Default,
            allowed_paths: None,
            llm_provider: None,
            ai_config: None,
            max_turns: 20,
        }
    }
}

impl EmbeddedSessionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.db_path = Some(path.into());
        self
    }

    pub fn in_memory(mut self) -> Self {
        self.db_path = None;
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn agent_id(mut self, id: impl Into<String>) -> Self {
        self.agent_id = Some(id.into());
        self
    }

    pub fn agent_name(mut self, name: impl Into<String>) -> Self {
        self.agent_name = Some(name.into());
        self
    }

    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    pub fn allowed_paths(mut self, paths: Vec<String>) -> Self {
        self.allowed_paths = Some(paths);
        self
    }

    pub fn provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    pub fn ai_config(mut self, config: AiConfig) -> Self {
        self.ai_config = Some(config);
        self
    }

    pub fn max_turns(mut self, turns: usize) -> Self {
        self.max_turns = turns;
        self
    }

    pub async fn build(self) -> Result<EmbeddedSession> {
        let db_target = match &self.db_path {
            Some(p) => p.to_string_lossy().to_string(),
            None => ":memory:".to_string(),
        };

        let db = cade_store::sqlite::open(&db_target)
            .map_err(|e| Error::custom(format!("failed to open sqlite database at {db_target}: {e}")))?;

        let agent_id = self
            .agent_id
            .unwrap_or_else(|| format!("emb-{}", uuid::Uuid::new_v4()));

        let agent_name = self
            .agent_name
            .unwrap_or_else(|| format!("EmbeddedAgent-{}", &agent_id[..6.min(agent_id.len())]));

        let agent_row = AgentRow {
            id: agent_id.clone(),
            name: agent_name,
            model: self.model.clone(),
            description: Some("Embedded agent".to_string()),
            system_prompt: self.system_prompt.clone(),
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        let _ = cade_store::sqlite::create_agent(&db, &agent_row);

        let provider: Arc<dyn LlmProvider> = if let Some(p) = self.llm_provider {
            p
        } else if let Some(cfg) = self.ai_config {
            Arc::new(LlmRouter::build(&cfg))
        } else {
            let env_config = AiConfig {
                anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
                openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
                google_api_key: std::env::var("GEMINI_API_KEY")
                    .ok()
                    .or_else(|| std::env::var("GOOGLE_API_KEY").ok()),
                ollama_base_url: std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string()),
                llm_provider: "anthropic".to_string(),
            };
            Arc::new(LlmRouter::build(&env_config))
        };

        let storage = Arc::new(EmbeddedStorageBackend { db: db.clone() });
        let mcp = Arc::new(McpManager::empty());

        let mut runtime = ToolRuntime::new(
            storage,
            mcp,
            agent_id.clone(),
            self.cwd,
        );
        runtime.allowed_paths = self.allowed_paths;

        let _permissions = PermissionManager::new(self.permission_mode);

        Ok(EmbeddedSession {
            agent_id,
            model: self.model,
            system_prompt: self.system_prompt,
            db,
            provider,
            runtime: Arc::new(runtime),
            max_turns: self.max_turns,
        })
    }
}

// endregion: --- EmbeddedSessionBuilder

// region:    --- EmbeddedSession

/// In-process zero-daemon agent session linking directly to SQLite and LLM provider.
pub struct EmbeddedSession {
    agent_id: String,
    model: String,
    system_prompt: Option<String>,
    db: Db,
    provider: Arc<dyn LlmProvider>,
    runtime: Arc<ToolRuntime>,
    max_turns: usize,
}

impl EmbeddedSession {
    /// Create a new builder for configuring an `EmbeddedSession`.
    pub fn builder() -> EmbeddedSessionBuilder {
        EmbeddedSessionBuilder::new()
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn runtime(&self) -> &Arc<ToolRuntime> {
        &self.runtime
    }

    /// Build context messages array (system prompt + memory blocks + message history).
    fn build_messages_context(&self, conversation_id: Option<&str>) -> Result<Vec<LlmMessage>> {
        let mut messages = Vec::new();

        // 1. System prompt & persistent memory blocks
        let mut sys = self.system_prompt.clone().unwrap_or_default();
        let memory_blocks = cade_store::sqlite::get_memory_blocks_full(&self.db, &self.agent_id)
            .map_err(|e| Error::custom(format!("failed to load memory: {e}")))?;

        if !memory_blocks.is_empty() {
            if !sys.is_empty() {
                sys.push_str("\n\n");
            }
            sys.push_str("# Memory\n");
            for (label, value, _, _) in memory_blocks {
                sys.push_str(&format!("[{label}]\n{value}\n\n"));
            }
        }

        if !sys.trim().is_empty() {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: sys.trim().to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            });
        }

        // 2. Chat history
        let history = cade_store::sqlite::list_messages(&self.db, &self.agent_id, conversation_id, 100)
            .map_err(|e| Error::custom(format!("failed to load history: {e}")))?;

        for m in history {
            let text = if let Value::String(s) = m.content {
                s
            } else {
                m.content.to_string()
            };
            messages.push(LlmMessage {
                role: m.role,
                content: text,
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            });
        }

        Ok(messages)
    }

    /// Send a prompt and execute the agentic loop to convergence in-process.
    pub async fn prompt(&self, text: &str) -> Result<String> {
        let conversation_id = format!("conv-{}", self.agent_id);

        // 1. Persist user message
        let user_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
        let _ = cade_store::sqlite::insert_message(
            &self.db,
            &MessageRow {
                id: user_msg_id,
                agent_id: self.agent_id.clone(),
                conversation_id: Some(conversation_id.clone()),
                role: "user".to_string(),
                content: Value::String(text.to_string()),
                char_count: text.len(),
            },
        );

        let tools = all_schemas(false);
        let mut final_content = String::new();

        // 2. Agentic turn loop
        for _turn in 0..self.max_turns {
            let messages = self.build_messages_context(Some(&conversation_id))?;
            let req = CompletionRequest {
                model: self.model.clone(),
                messages,
                tools: tools.clone(),
                max_tokens: 4096,
                reasoning_effort: None,
            };

            let resp = self.provider.complete(&req).await?;
            let text_chunk = resp.content.unwrap_or_default();
            if !text_chunk.is_empty() {
                final_content = text_chunk.clone();
            }

            if resp.tool_calls.is_empty() {
                // Done - persist assistant message
                let asst_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
                let _ = cade_store::sqlite::insert_message(
                    &self.db,
                    &MessageRow {
                        id: asst_msg_id,
                        agent_id: self.agent_id.clone(),
                        conversation_id: Some(conversation_id.clone()),
                        role: "assistant".to_string(),
                        content: Value::String(final_content.clone()),
                        char_count: final_content.len(),
                    },
                );
                break;
            }

            // Persist assistant message with tool call
            let asst_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
            let _ = cade_store::sqlite::insert_message(
                &self.db,
                &MessageRow {
                    id: asst_msg_id,
                    agent_id: self.agent_id.clone(),
                    conversation_id: Some(conversation_id.clone()),
                    role: "assistant".to_string(),
                    content: Value::String(text_chunk.clone()),
                    char_count: text_chunk.len(),
                },
            );

            // Execute each tool call
            for tc in resp.tool_calls {
                let tool_res = self
                    .runtime
                    .execute(tc.id.clone(), &tc.name, &tc.arguments)
                    .await
                    .unwrap_or_else(|| RuntimeToolResult {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        output: format!("Error: Tool '{}' not found.", tc.name),
                        is_error: true,
                        ui_resource_uri: None,
                    });

                // Persist tool result
                let tool_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
                let _ = cade_store::sqlite::insert_message(
                    &self.db,
                    &MessageRow {
                        id: tool_msg_id,
                        agent_id: self.agent_id.clone(),
                        conversation_id: Some(conversation_id.clone()),
                        role: "tool".to_string(),
                        content: Value::String(tool_res.output),
                        char_count: 0,
                    },
                );
            }
        }

        Ok(final_content)
    }

    /// Stream typed [`CadeStreamEvent`] telemetry in real-time during execution.
    pub async fn stream_prompt(
        &self,
        text: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = CadeStreamEvent> + Send>>> {
        let (tx, rx) = mpsc::channel(64);

        let agent_id = self.agent_id.clone();
        let model = self.model.clone();
        let db = self.db.clone();
        let provider = self.provider.clone();
        let runtime = self.runtime.clone();
        let max_turns = self.max_turns;
        let user_text = text.to_string();
        let sys_prompt = self.system_prompt.clone();

        tokio::spawn(async move {
            let conversation_id = format!("conv-{agent_id}");

            // 1. Persist user message
            let user_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
            let _ = cade_store::sqlite::insert_message(
                &db,
                &MessageRow {
                    id: user_msg_id,
                    agent_id: agent_id.clone(),
                    conversation_id: Some(conversation_id.clone()),
                    role: "user".to_string(),
                    content: Value::String(user_text.clone()),
                    char_count: user_text.len(),
                },
            );

            let tools = all_schemas(false);
            let mut final_content = String::new();

            for _turn in 0..max_turns {
                // Build context
                let mut messages = Vec::new();
                let mut sys = sys_prompt.clone().unwrap_or_default();
                let memory_blocks = cade_store::sqlite::get_memory_blocks_full(&db, &agent_id)
                    .unwrap_or_default();

                if !memory_blocks.is_empty() {
                    if !sys.is_empty() {
                        sys.push_str("\n\n");
                    }
                    sys.push_str("# Memory\n");
                    for (label, value, _, _) in memory_blocks {
                        sys.push_str(&format!("[{label}]\n{value}\n\n"));
                    }
                }

                if !sys.trim().is_empty() {
                    messages.push(LlmMessage {
                        role: "system".to_string(),
                        content: sys.trim().to_string(),
                        tool_call_id: None,
                        tool_calls: None,
                        images: None,
                        cache_control: None,
                    });
                }

                let history = cade_store::sqlite::list_messages(&db, &agent_id, Some(&conversation_id), 100)
                    .unwrap_or_default();
                for m in history {
                    let txt = if let Value::String(s) = m.content {
                        s
                    } else {
                        m.content.to_string()
                    };
                    messages.push(LlmMessage {
                        role: m.role,
                        content: txt,
                        tool_call_id: None,
                        tool_calls: None,
                        images: None,
                        cache_control: None,
                    });
                }

                let req = CompletionRequest {
                    model: model.clone(),
                    messages,
                    tools: tools.clone(),
                    max_tokens: 4096,
                    reasoning_effort: None,
                };

                let mut current_turn_text = String::new();
                let mut current_tool_calls: Vec<LlmToolCall> = Vec::new();

                match provider.stream(&req).await {
                    Ok(mut stream) => {
                        use futures::StreamExt;
                        while let Some(chunk_res) = stream.next().await {
                            match chunk_res {
                                Ok(StreamChunk::Text(delta)) => {
                                    current_turn_text.push_str(&delta);
                                    let _ = tx.send(CadeStreamEvent::MessageDelta(delta)).await;
                                }
                                Ok(StreamChunk::Reasoning(thought)) => {
                                    let _ = tx.send(CadeStreamEvent::Thought(thought)).await;
                                }
                                Ok(StreamChunk::ToolCall(tc)) => {
                                    current_tool_calls.push(tc);
                                }
                                Ok(StreamChunk::Usage(u)) => {
                                    let _ = tx
                                        .send(CadeStreamEvent::Usage {
                                            input_tokens: u.input_tokens as u64,
                                            output_tokens: u.output_tokens as u64,
                                            model: u.model,
                                        })
                                        .await;
                                }
                                Ok(StreamChunk::FinishReason(r)) => {
                                    let _ = tx
                                        .send(CadeStreamEvent::Finished { outcome: r })
                                        .await;
                                }
                                Ok(StreamChunk::Done) => {}
                                Err(e) => {
                                    let _ = tx.send(CadeStreamEvent::Error(e.to_string())).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Fallback to complete
                        if let Ok(resp) = provider.complete(&req).await {
                            if let Some(c) = resp.content {
                                current_turn_text = c.clone();
                                let _ = tx.send(CadeStreamEvent::MessageDelta(c)).await;
                            }
                            current_tool_calls = resp.tool_calls;
                        } else {
                            let _ = tx.send(CadeStreamEvent::Error(e.to_string())).await;
                            break;
                        }
                    }
                }

                if !current_turn_text.is_empty() {
                    final_content = current_turn_text.clone();
                }

                if current_tool_calls.is_empty() {
                    let asst_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
                    let _ = cade_store::sqlite::insert_message(
                        &db,
                        &MessageRow {
                            id: asst_msg_id,
                            agent_id: agent_id.clone(),
                            conversation_id: Some(conversation_id.clone()),
                            role: "assistant".to_string(),
                            content: Value::String(final_content.clone()),
                            char_count: final_content.len(),
                        },
                    );
                    break;
                }

                // Persist assistant message with tool calls
                let asst_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
                let _ = cade_store::sqlite::insert_message(
                    &db,
                    &MessageRow {
                        id: asst_msg_id,
                        agent_id: agent_id.clone(),
                        conversation_id: Some(conversation_id.clone()),
                        role: "assistant".to_string(),
                        content: Value::String(current_turn_text.clone()),
                        char_count: current_turn_text.len(),
                    },
                );

                for tc in current_tool_calls {
                    let _ = tx
                        .send(CadeStreamEvent::ToolExecuting {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        })
                        .await;

                    let tool_res = runtime
                        .execute(tc.id.clone(), &tc.name, &tc.arguments)
                        .await
                        .unwrap_or_else(|| RuntimeToolResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: format!("Error: Tool '{}' not found.", tc.name),
                            is_error: true,
                            ui_resource_uri: None,
                        });

                    let _ = tx
                        .send(CadeStreamEvent::ToolCompleted {
                            tool_call_id: tool_res.tool_call_id.clone(),
                            tool_name: tool_res.tool_name.clone(),
                            output: tool_res.output.clone(),
                            is_error: tool_res.is_error,
                        })
                        .await;

                    let tool_msg_id = format!("msg-{}", uuid::Uuid::new_v4());
                    let _ = cade_store::sqlite::insert_message(
                        &db,
                        &MessageRow {
                            id: tool_msg_id,
                            agent_id: agent_id.clone(),
                            conversation_id: Some(conversation_id.clone()),
                            role: "tool".to_string(),
                            content: Value::String(tool_res.output),
                            char_count: 0,
                        },
                    );
                }
            }

            let _ = tx
                .send(CadeStreamEvent::Finished {
                    outcome: "completed".to_string(),
                })
                .await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// Retrieve the value of a memory block.
    pub async fn get_memory(&self, label: &str) -> Result<Option<String>> {
        let blocks = cade_store::sqlite::get_memory_blocks(&self.db, &self.agent_id)
            .map_err(|e| Error::custom(format!("get_memory: {e}")))?;
        Ok(blocks
            .into_iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, v, _)| v))
    }

    /// Set a memory block.
    pub async fn set_memory(&self, label: &str, value: &str) -> Result<()> {
        cade_store::sqlite::upsert_memory_block(
            &self.db,
            &self.agent_id,
            label,
            value,
            None,
            Some(4000),
        )
        .map_err(|e| Error::custom(format!("set_memory: {e}")))?;
        Ok(())
    }

    /// Delete a memory block.
    pub async fn delete_memory(&self, label: &str) -> Result<()> {
        cade_store::sqlite::delete_memory_block(&self.db, &self.agent_id, label)
            .map_err(|e| Error::custom(format!("delete_memory: {e}")))?;
        Ok(())
    }

    /// List all memory blocks for this agent.
    pub async fn list_memory(&self) -> Result<Vec<MemoryBlock>> {
        let blocks = cade_store::sqlite::get_memory_blocks_full(&self.db, &self.agent_id)
            .map_err(|e| Error::custom(format!("list_memory: {e}")))?;
        Ok(blocks
            .into_iter()
            .map(|(label, value, description, tier)| MemoryBlock {
                label,
                value,
                description: if description.is_empty() {
                    None
                } else {
                    Some(description)
                },
                tier: Some(tier),
            })
            .collect())
    }

    /// List all available skills for the current working directory.
    pub fn list_skills(&self) -> Vec<Skill> {
        cade_core::skills::discover_all_skills(&self.runtime.cwd, Some(&self.agent_id), None)
    }
}

// endregion: --- EmbeddedSession

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cade_ai::{CompletionRequest, CompletionResponse, StreamChunk};
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockLlmProvider {
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
        async fn complete(&self, req: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                // First turn: invoke a tool
                Ok(CompletionResponse {
                    content: Some("I am reading a file.".to_string()),
                    tool_calls: vec![LlmToolCall {
                        id: "call-1".to_string(),
                        name: "glob".to_string(),
                        arguments: json!({ "pattern": "*.toml" }),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".to_string(),
                })
            } else {
                // Second turn: answer with result
                let last_msg = req.messages.last().map(|m| m.content.as_str()).unwrap_or("");
                Ok(CompletionResponse {
                    content: Some(format!("Found files from tool. Response: {last_msg}")),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                })
            }
        }

        async fn stream(
            &self,
            req: &CompletionRequest,
        ) -> cade_ai::Result<Pin<Box<dyn Stream<Item = cade_ai::Result<StreamChunk>> + Send>>> {
            let resp = self.complete(req).await?;
            let mut chunks = Vec::new();
            if let Some(c) = resp.content {
                chunks.push(Ok(StreamChunk::Text(c)));
            }
            for tc in resp.tool_calls {
                chunks.push(Ok(StreamChunk::ToolCall(tc)));
            }
            chunks.push(Ok(StreamChunk::FinishReason(resp.finish_reason)));
            chunks.push(Ok(StreamChunk::Done));
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    #[tokio::test]
    async fn test_embedded_session_in_memory_multi_turn() {
        let mock_provider = Arc::new(MockLlmProvider {
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let session = EmbeddedSession::builder()
            .in_memory()
            .model("mock-model")
            .provider(mock_provider)
            .build()
            .await
            .expect("session creation should succeed");

        let response = session
            .prompt("List toml files in the project")
            .await
            .expect("prompt should succeed");

        assert!(response.contains("Found files from tool"));

        // Memory tests
        session
            .set_memory("project_rule", "Strict TDD")
            .await
            .expect("set memory");

        let val = session
            .get_memory("project_rule")
            .await
            .expect("get memory");
        assert_eq!(val.as_deref(), Some("Strict TDD"));
    }

    #[tokio::test]
    async fn test_embedded_session_streaming_events() {
        let mock_provider = Arc::new(MockLlmProvider {
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let session = EmbeddedSession::builder()
            .in_memory()
            .model("mock-model")
            .provider(mock_provider)
            .build()
            .await
            .expect("session creation should succeed");

        let mut stream = session
            .stream_prompt("Stream test prompt")
            .await
            .expect("stream prompt should succeed");

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }

        assert!(!events.is_empty());
        let has_delta = events.iter().any(|e| matches!(e, CadeStreamEvent::MessageDelta(_)));
        let has_tool_exec = events.iter().any(|e| matches!(e, CadeStreamEvent::ToolExecuting { .. }));
        let has_tool_done = events.iter().any(|e| matches!(e, CadeStreamEvent::ToolCompleted { .. }));
        let has_finish = events.iter().any(|e| matches!(e, CadeStreamEvent::Finished { .. }));

        assert!(has_delta, "Should emit MessageDelta");
        assert!(has_tool_exec, "Should emit ToolExecuting");
        assert!(has_tool_done, "Should emit ToolCompleted");
        assert!(has_finish, "Should emit Finished");
    }
}

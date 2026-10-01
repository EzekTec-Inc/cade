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
use cade_agent::tools::ToolRuntime;
use cade_ai::{AiConfig, LlmProvider, LlmRouter};
use cade_core::permissions::PermissionMode;
use cade_core::skills::Skill;
use cade_server_lib::server::api::run::runtime::{
    RunExecutionOptions, RunRequest, ServerAgentRuntime,
};
use cade_store::Db;
use cade_store::sqlite::AgentRow;

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
        cade_store::sqlite::upsert_memory_block(&self.db, agent_id, label, value, desc, limit)
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
        let blocks = cade_store::sqlite::get_memory_blocks(&self.db, agent_id).unwrap_or_default();
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
            .unwrap_or_default()
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
            Err(e) => Err(cade_agent::Error::custom(format!(
                "Failed to store artifact: {e}"
            ))),
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
            &self.db, agent_id, label, kind, reference, excerpt, 1.0,
        )
        .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(())
    }

    async fn trigger_reflect(
        &self,
        _agent_id: &str,
        _focus: Option<&str>,
    ) -> cade_agent::Result<()> {
        Ok(())
    }

    async fn install_plugin(
        &self,
        _agent_id: &str,
        _url: &str,
        _plugin_id: &str,
    ) -> cade_agent::Result<String> {
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
            .unwrap_or_default()
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
            Err(e) => Err(cade_agent::Error::custom(format!(
                "Failed to create checkpoint: {e}"
            ))),
        }
    }

    async fn list_checkpoints(&self, agent_id: &str) -> cade_agent::Result<Vec<Value>> {
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

    async fn get_checkpoint(
        &self,
        agent_id: &str,
        checkpoint_id: &str,
    ) -> cade_agent::Result<Value> {
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

    async fn restore_checkpoint(
        &self,
        _agent_id: &str,
        _checkpoint_id: &str,
    ) -> cade_agent::Result<()> {
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

    async fn message_agent(
        &self,
        _agent_id: &str,
        _target: &str,
        _message: &str,
    ) -> cade_agent::Result<String> {
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
    model: Option<String>,
    agent_id: Option<String>,
    agent_name: Option<String>,
    system_prompt: Option<String>,
    cwd: PathBuf,
    permission_mode: PermissionMode,
    allowed_paths: Option<Vec<String>>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ai_config: Option<AiConfig>,
    max_turns: usize,
    permissions: Option<cade_core::settings::PermissionSettings>,
    execution: Option<cade_core::settings::ExecutionProfile>,
    reasoning_effort: Option<String>,
}

impl Default for EmbeddedSessionBuilder {
    fn default() -> Self {
        Self {
            db_path: None,
            model: None,
            agent_id: None,
            agent_name: None,
            system_prompt: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            permission_mode: PermissionMode::Default,
            allowed_paths: None,
            llm_provider: None,
            ai_config: None,
            max_turns: 20,
            permissions: None,
            execution: None,
            reasoning_effort: None,
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
        self.model = Some(model.into());
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

    pub fn permissions(mut self, permissions: cade_core::settings::PermissionSettings) -> Self {
        self.permissions = Some(permissions);
        self
    }

    pub fn execution(mut self, execution: cade_core::settings::ExecutionProfile) -> Self {
        self.execution = Some(execution);
        self
    }

    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(effort.into());
        self
    }

    pub async fn build(self) -> Result<EmbeddedSession> {
        let db_target = match &self.db_path {
            Some(p) => p.to_string_lossy().to_string(),
            None => ":memory:".to_string(),
        };

        let db = cade_store::sqlite::open(&db_target).map_err(|e| {
            Error::custom(format!(
                "failed to open sqlite database at {db_target}: {e}"
            ))
        })?;

        let agent_id = self
            .agent_id
            .unwrap_or_else(|| format!("emb-{}", uuid::Uuid::new_v4()));

        let agent_name = self
            .agent_name
            .unwrap_or_else(|| format!("EmbeddedAgent-{}", &agent_id[..6.min(agent_id.len())]));

        let existing_agent = cade_store::sqlite::get_agent(&db, &agent_id)
            .map_err(|error| Error::custom(error.to_string()))?;
        let provider: Arc<dyn LlmProvider> = if let Some(provider) = &self.llm_provider {
            Arc::clone(provider)
        } else if let Some(config) = &self.ai_config {
            Arc::new(LlmRouter::build(config))
        } else {
            Arc::new(LlmRouter::build(&AiConfig::from_env()))
        };
        let model = match self
            .model
            .clone()
            .or_else(|| existing_agent.as_ref().map(|agent| agent.model.clone()))
            .or_else(|| {
                self.llm_provider
                    .as_ref()
                    .and_then(|provider| provider.default_model())
            }) {
            Some(model) => model,
            None => cade_ai::provider_registry::ProviderRegistry::configured()
                .configured_default_model(
                    self.ai_config
                        .as_ref()
                        .map(|config| config.llm_provider.as_str()),
                )
                .map_err(|error| Error::custom(error.to_string()))?,
        };
        provider
            .validate_model(&model)
            .map_err(|error| Error::custom(error.to_string()))?;
        let agent_row = AgentRow {
            id: agent_id.clone(),
            name: agent_name,
            model: model.clone(),
            description: Some("Embedded agent".to_string()),
            system_prompt: self.system_prompt.clone(),
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        if existing_agent.is_none() {
            cade_store::sqlite::create_agent(&db, &agent_row)
                .map_err(|e| Error::custom(e.to_string()))?;
        } else if self.model.is_some()
            && existing_agent
                .as_ref()
                .is_some_and(|agent| agent.model != model)
        {
            cade_store::sqlite::update_agent_model(&db, &agent_id, &model)
                .map_err(|error| Error::custom(error.to_string()))?;
        }
        let conversation_id = cade_store::sqlite::create_conversation(&db, &agent_id, "")
            .map_err(|e| Error::custom(e.to_string()))?
            .id;

        let env_config = AiConfig::from_env();
        let router_instance = if let Some(cfg) = &self.ai_config {
            LlmRouter::build(cfg)
        } else {
            LlmRouter::build(&env_config)
        };
        let router = Arc::new(tokio::sync::RwLock::new(router_instance));
        let config = Arc::new(cade_server_lib::server::config::ServerConfig {
            default_model: model.clone(),
            ..Default::default()
        });
        let app_state = cade_server_lib::server::state::AppState::new_in_process(
            db.clone(),
            provider.clone(),
            router,
            config,
            Arc::new(McpManager::empty()),
        );
        let agent_runtime = ServerAgentRuntime::new(app_state);
        let request = RunRequest {
            agent_id: agent_id.clone(),
            conversation_id: Some(conversation_id.clone()),
            input: String::new(),
            permission_mode: None,
        };
        let options = RunExecutionOptions {
            cwd: Some(self.cwd),
            allowed_paths: self.allowed_paths,
            permission_mode: Some(self.permission_mode.to_string()),
            permissions: self.permissions,
            execution: self.execution,
            reasoning_effort: self.reasoning_effort,
            max_turns: Some(self.max_turns),
            ..Default::default()
        };
        let runtime = agent_runtime
            .prepare_tool_runtime(&request, options.clone())
            .map_err(|e| Error::custom(e.to_string()))?;
        let agent_runtime = agent_runtime.with_execution_options(RunExecutionOptions {
            cwd: Some(runtime.cwd.clone()),
            allowed_paths: None,
            execution: None,
            tool_runtime: Some(runtime.clone()),
            ..options
        });

        Ok(EmbeddedSession {
            agent_id,
            model,
            conversation_id,
            db,
            runtime,
            agent_runtime,
        })
    }
}

// endregion: --- EmbeddedSessionBuilder

// region:    --- EmbeddedSession

/// In-process zero-daemon agent session linking directly to SQLite and LLM provider.
pub struct EmbeddedSession {
    agent_id: String,
    model: String,
    conversation_id: String,
    db: Db,
    runtime: Arc<ToolRuntime>,
    agent_runtime: cade_server_lib::server::api::run::runtime::ServerAgentRuntime,
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

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    /// Accept a durable in-process run and expose its id and ordered events.
    pub async fn start_run(
        &self,
        text: &str,
    ) -> Result<cade_server_lib::server::api::run::runtime::RunHandle> {
        self.agent_runtime
            .try_start(RunRequest {
                agent_id: self.agent_id.clone(),
                conversation_id: Some(self.conversation_id.clone()),
                input: text.into(),
                permission_mode: None,
            })
            .await
            .map_err(|error| Error::custom(error.to_string()))
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<bool> {
        let run = cade_store::sqlite::get_run(&self.db, run_id)
            .map_err(|error| Error::custom(error.to_string()))?
            .ok_or_else(|| Error::custom("Run not found"))?;
        if run.agent_id != self.agent_id
            || run.conversation_id.as_deref() != Some(self.conversation_id.as_str())
        {
            return Err(Error::custom("Run does not belong to this session"));
        }
        cade_store::sqlite::request_run_cancellation(&self.db, run_id)
            .map_err(|error| Error::custom(error.to_string()))
    }

    /// Send a prompt and execute the agentic loop to convergence in-process.
    pub async fn prompt(&self, text: &str) -> Result<String> {
        use futures::StreamExt;
        let mut stream = self.stream_prompt(text).await?;
        let mut final_content = String::new();
        while let Some(event) = stream.next().await {
            match event {
                CadeStreamEvent::MessageDelta(delta) => {
                    final_content.push_str(&delta);
                }
                CadeStreamEvent::Error(err) => {
                    return Err(Error::custom(err));
                }
                CadeStreamEvent::Finished { outcome } => {
                    use cade_agent::agent::client::RunOutcome;
                    return match RunOutcome::from_status(&outcome) {
                        Some(RunOutcome::Completed) => Ok(final_content),
                        _ => Err(Error::custom(format!("Run {outcome}"))),
                    };
                }
                _ => {}
            }
        }
        Err(Error::custom(
            "Run observation incomplete: no terminal outcome",
        ))
    }

    /// Stream typed [`CadeStreamEvent`] telemetry in real-time during execution.
    pub async fn stream_prompt(
        &self,
        text: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = CadeStreamEvent> + Send>>> {
        let (tx, rx) = mpsc::channel(64);
        let handle = self.start_run(text).await?;
        let db = self.db.clone();

        tokio::spawn(async move {
            let run_id = handle.run_id;
            let mut stream = handle.events;
            let mut cursor = -1;
            while let Some(res) = stream.recv().await {
                let Ok(env) = res;
                let data = env.data;
                let trimmed = data.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "[DONE]" {
                    break;
                }
                let sequence = serde_json::from_str::<Value>(trimmed)
                    .ok()
                    .and_then(|value| value["seq_id"].as_i64());
                if let Some(sequence) = sequence {
                    if sequence <= cursor {
                        continue;
                    }
                    if sequence > cursor + 1 {
                        if !replay_embedded_events(&db, &run_id, &mut cursor, &tx).await {
                            return;
                        }
                        if cursor < sequence {
                            let _ = tx.send(CadeStreamEvent::Error(format!("Run {run_id} observation incomplete: missing journal events after {cursor}"))).await;
                            return;
                        }
                        continue;
                    }
                    cursor = sequence;
                }
                if !forward_embedded_event(&tx, trimmed).await {
                    return;
                }
            }
            // Tail replay covers a terminal frame skipped under backpressure.
            if !replay_embedded_events(&db, &run_id, &mut cursor, &tx).await {
                return;
            }
            let status = cade_store::sqlite::get_run(&db, &run_id);
            match status {
                Ok(Some(run))
                    if cade_agent::agent::client::RunOutcome::from_status(&run.status)
                        .is_some() =>
                {
                    // Re-read after terminal status: a final event may have been
                    // committed between the previous snapshot and status lookup.
                    if replay_embedded_events(&db, &run_id, &mut cursor, &tx).await {
                        let _ = tx
                            .send(CadeStreamEvent::Finished {
                                outcome: run.status,
                            })
                            .await;
                    }
                }
                other => {
                    let detail = other
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "no terminal outcome".into());
                    let _ = tx
                        .send(CadeStreamEvent::Error(format!(
                            "Run {run_id} observation incomplete: {detail}"
                        )))
                        .await;
                }
            }
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

async fn forward_embedded_event(tx: &mpsc::Sender<CadeStreamEvent>, data: &str) -> bool {
    let event = match serde_json::from_str::<cade_api_types::StreamEvent>(data) {
        Ok(event) => event,
        Err(error) => {
            let _ = tx
                .send(CadeStreamEvent::Error(format!(
                    "Invalid Run event: {error}"
                )))
                .await;
            return false;
        }
    };
    if let Some(error) =
        cade_agent::agent::client::run_observation_error(event.msg_type(), &event.data)
    {
        let _ = tx.send(CadeStreamEvent::Error(error)).await;
        return false;
    }
    if event.msg_type() == "run_done"
        && event.data["status"]
            .as_str()
            .and_then(cade_agent::agent::client::RunOutcome::from_status)
            .is_none()
    {
        let _ = tx
            .send(CadeStreamEvent::Error("Invalid Run terminal status".into()))
            .await;
        return false;
    }
    let terminal = event.msg_type() == "run_done";
    if let Some(event) = CadeStreamEvent::from_stream_event(&event) {
        return tx.send(event).await.is_ok() && !terminal;
    }
    true
}

async fn replay_embedded_events(
    db: &Db,
    run_id: &str,
    cursor: &mut i64,
    tx: &mpsc::Sender<CadeStreamEvent>,
) -> bool {
    let rows = match cade_store::sqlite::run_events_after(db, run_id, *cursor) {
        Ok(rows) => rows,
        Err(error) => {
            let _ = tx
                .send(CadeStreamEvent::Error(format!(
                    "Run event replay failed: {error}"
                )))
                .await;
            return false;
        }
    };
    for (sequence, data) in rows {
        if sequence != *cursor + 1 {
            let _ = tx
                .send(CadeStreamEvent::Error(format!(
                    "Run {run_id} observation incomplete: journal gap after {cursor}"
                )))
                .await;
            return false;
        }
        let mut payload = match serde_json::from_str::<Value>(&data) {
            Ok(payload) => payload,
            Err(error) => {
                let _ = tx
                    .send(CadeStreamEvent::Error(format!(
                        "Invalid replay for Run {run_id}: {error}"
                    )))
                    .await;
                return false;
            }
        };
        payload["run_id"] = run_id.into();
        payload["seq_id"] = sequence.into();
        if !forward_embedded_event(tx, &payload.to_string()).await {
            return false;
        }
        *cursor = sequence;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cade_ai::{CompletionRequest, CompletionResponse, LlmToolCall, StreamChunk};
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
                let last_msg = req
                    .messages
                    .last()
                    .map(|m| m.content.as_str())
                    .unwrap_or("");
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
        ) -> cade_ai::Result<Pin<Box<dyn Stream<Item = cade_ai::Result<StreamChunk>> + Send>>>
        {
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
    async fn configured_custom_provider_default_is_selected_by_embedded_factory() {
        let definitions = cade_ai::provider_registry::ProviderRegistry::from_json(
            &json!([{
                "name":"office", "aliases":["office-alt"], "kind":"openai-compatible",
                "chat_url":"http://127.0.0.1:1/v1", "default_model":"tenant/sdk-deployment"
            }])
            .to_string(),
        )
        .unwrap();
        let mut router = LlmRouter::empty("office".into(), Arc::new(Default::default()))
            .with_provider_registry(definitions);
        router.add_provider(
            "office-alt".into(),
            Arc::new(MockLlmProvider {
                call_count: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let workspace = Workspace::new();
        let session = EmbeddedSession::builder()
            .in_memory()
            .cwd(&workspace.0)
            .provider(Arc::new(router))
            .build()
            .await
            .unwrap();
        assert_eq!(session.model(), "office/tenant/sdk-deployment");
        assert_eq!(
            cade_store::sqlite::get_agent(session.db(), session.agent_id())
                .unwrap()
                .unwrap()
                .model,
            "office/tenant/sdk-deployment"
        );
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
        let has_delta = events
            .iter()
            .any(|e| matches!(e, CadeStreamEvent::MessageDelta(_)));
        let has_tool_exec = events
            .iter()
            .any(|e| matches!(e, CadeStreamEvent::ToolExecuting { .. }));
        let has_tool_done = events
            .iter()
            .any(|e| matches!(e, CadeStreamEvent::ToolCompleted { .. }));
        let has_finish = events
            .iter()
            .any(|e| matches!(e, CadeStreamEvent::Finished { .. }));

        assert!(has_delta, "Should emit MessageDelta");
        assert!(has_tool_exec, "Should emit ToolExecuting");
        assert!(has_tool_done, "Should emit ToolCompleted");
        assert!(has_finish, "Should emit Finished");
    }

    struct Workspace(std::path::PathBuf);
    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cade-embedded-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FileProvider {
        calls: AtomicUsize,
        write: bool,
        reasoning: std::sync::Mutex<Vec<Option<String>>>,
    }
    #[async_trait]
    impl LlmProvider for FileProvider {
        async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
            unreachable!()
        }
        async fn stream(
            &self,
            request: &CompletionRequest,
        ) -> cade_ai::Result<Pin<Box<dyn Stream<Item = cade_ai::Result<StreamChunk>> + Send>>>
        {
            self.reasoning
                .lock()
                .unwrap()
                .push(request.reasoning_effort.clone());
            let chunks = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                vec![
                    Ok(StreamChunk::ToolCall(LlmToolCall {
                        id: "file-call".into(),
                        name: if self.write {
                            "write_file"
                        } else {
                            "read_file"
                        }
                        .into(),
                        arguments: json!({"path":"file.txt", "content":"changed"}),
                        thought_signature: None,
                    })),
                    Ok(StreamChunk::Done),
                ]
            } else {
                let result = request
                    .messages
                    .iter()
                    .filter(|message| message.role == "tool")
                    .map(|message| message.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                vec![Ok(StreamChunk::Text(result)), Ok(StreamChunk::Done)]
            };
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    #[tokio::test]
    async fn candidate1_embedded_workspace_accessor_permissions_and_allowed_paths_are_effective() {
        for (mode, paths, write, should_error) in [
            (PermissionMode::Default, None, false, false),
            (PermissionMode::Plan, None, true, true),
            (
                PermissionMode::AcceptEdits,
                Some(vec!["elsewhere".into()]),
                true,
                true,
            ),
            (PermissionMode::AcceptEdits, None, true, false),
        ] {
            let workspace = Workspace::new();
            std::fs::write(workspace.0.join("file.txt"), "unique workspace contents").unwrap();
            let provider = Arc::new(FileProvider {
                calls: AtomicUsize::new(0),
                write,
                reasoning: Default::default(),
            });
            let mut builder = EmbeddedSession::builder()
                .in_memory()
                .model("test")
                .provider(provider.clone())
                .cwd(&workspace.0)
                .permission_mode(mode)
                .permissions(Default::default())
                .execution(Default::default())
                .reasoning_effort("high");
            if let Some(paths) = paths {
                builder = builder.allowed_paths(paths);
            }
            let session = builder.build().await.unwrap();
            assert_eq!(session.runtime().cwd, workspace.0);
            assert_eq!(
                session.runtime().conversation_id.as_deref(),
                Some(session.conversation_id())
            );
            let response = session.prompt("operate on file").await.unwrap();
            let rows = cade_store::sqlite::list_messages(
                session.db(),
                session.agent_id(),
                Some(session.conversation_id()),
                100,
            )
            .unwrap();
            let tool = rows.iter().find(|row| row.role == "tool").unwrap();
            assert_eq!(tool.content["is_error"], should_error, "{response}");
            assert_eq!(
                std::fs::read_to_string(workspace.0.join("file.txt")).unwrap(),
                if write && !should_error {
                    "changed"
                } else {
                    "unique workspace contents"
                }
            );
            if !write {
                assert!(response.contains("unique workspace contents"), "{response}");
            }
            assert!(
                provider
                    .reasoning
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|value| value.as_deref() == Some("high"))
            );
        }
    }

    #[tokio::test]
    async fn candidate1_embedded_max_turns_is_honored_by_the_canonical_loop() {
        let workspace = Workspace::new();
        std::fs::write(workspace.0.join("file.txt"), "contents").unwrap();
        let provider = Arc::new(FileProvider {
            calls: AtomicUsize::new(0),
            write: false,
            reasoning: Default::default(),
        });
        let session = EmbeddedSession::builder()
            .in_memory()
            .model("test")
            .provider(provider.clone())
            .cwd(&workspace.0)
            .permissions(Default::default())
            .execution(Default::default())
            .max_turns(1)
            .build()
            .await
            .unwrap();
        assert!(
            session
                .prompt("read file")
                .await
                .unwrap_err()
                .to_string()
                .contains("exceeded 1 turns")
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert!(
            EmbeddedSession::builder()
                .max_turns(0)
                .build()
                .await
                .is_err()
        );
    }
}

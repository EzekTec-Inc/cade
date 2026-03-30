/// Unified tool dispatch runtime.
///
/// `ToolRuntime` is the single point of truth for executing tools that do not
/// require interactive TUI state.  It handles:
///
/// - All memory tools (update_memory, memory_apply_patch, archival_*, search_*)
/// - Skill tools (load_skill, install_skill, run_skill_script, load_skill_ref)
/// - Native tools (bash, read_file, write_file, edit_file, grep, glob, desktop)
/// - MCP tools
///
/// Interactive-only tools (`run_subagent`, `ask_user_question`, `EnterPlanMode`,
/// `ExitPlanMode`) are NOT dispatched here; those remain in `repl.rs` which has
/// access to the TUI app handle.
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use cade_core::skills::discover_all_skills;

use cade_core::tool_ids::*;

use crate::agent::client::CadeClient;
use crate::backends::{ExecutionBackend, LocalBackend};
use crate::mcp::McpManager;
use crate::tools::git_checkpoint;
use crate::tools::{dispatch, memory};

// region:    --- Types

/// Result of a single tool execution.
#[derive(Debug, Clone)]
pub struct RuntimeToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

// endregion: --- Types

// region:    --- ToolRuntime

/// Shared context for dispatching tool calls.
///
/// Create once per session and reuse across turns.
pub struct ToolRuntime {
    pub client: Arc<CadeClient>,
    pub mcp: Arc<McpManager>,
    pub agent_id: String,
    pub cwd: PathBuf,
    /// Active conversation ID — used for tool execution logging context.
    pub conversation_id: Option<String>,
    /// When true, each tool execution is logged to the server asynchronously.
    pub log_executions: bool,
    /// Pluggable execution backend (local / docker / ssh / readonly).
    pub backend: Arc<dyn ExecutionBackend>,
}

impl ToolRuntime {
    // -- Constructor

    pub fn new(
        client: Arc<CadeClient>,
        mcp: Arc<McpManager>,
        agent_id: String,
        cwd: PathBuf,
    ) -> Self {
        Self {
            client,
            mcp,
            agent_id,
            cwd,
            conversation_id: None,
            log_executions: false,
            backend: Arc::new(LocalBackend),
        }
    }

    /// Convenience constructor that clones the client and wraps an MCP reference.
    pub fn from_refs(client: &CadeClient, mcp: &McpManager, agent_id: &str, cwd: PathBuf) -> Self {
        let _ = mcp;
        Self {
            client: Arc::new(client.clone()),
            mcp: Arc::new(McpManager::empty()),
            agent_id: agent_id.to_string(),
            cwd,
            conversation_id: None,
            log_executions: false,
            backend: Arc::new(LocalBackend),
        }
    }

    /// Set the active conversation ID (enables contextual tool execution logging).
    pub fn with_conversation(mut self, conv_id: Option<String>) -> Self {
        self.conversation_id = conv_id;
        self
    }

    /// Enable async tool execution logging to the server.
    pub fn with_logging(mut self) -> Self {
        self.log_executions = true;
        self
    }

    /// Set a custom execution backend (docker / ssh / readonly / etc).
    pub fn with_backend(mut self, backend: Arc<dyn ExecutionBackend>) -> Self {
        self.backend = backend;
        self
    }

    // -- Dispatch

    /// Dispatch a single tool call and return its output.
    ///
    /// Returns `None` for tools that this runtime does not handle (interactive
    /// tools that need TUI context — callers should intercept those first).
    pub async fn execute(
        &self,
        tool_call_id: String,
        tool_name: &str,
        args: &Value,
    ) -> Option<RuntimeToolResult> {
        // Normalise Gemini / Codex aliases back to canonical IDs.
        let canonical_owned: String = {
            use cade_core::toolsets::Toolset;
            use cade_core::toolsets::adapter::ToolSurfaceAdapter;
            let ga = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);
            ga.to_canonical(tool_name).to_string()
        };
        let canonical = canonical_owned.as_str();

        let t0 = std::time::Instant::now();
        let (output, is_error) = match canonical {
            // -- Memory tools (intercepted; use REST client)
            UPDATE_MEMORY => self.handle_update_memory(args).await,
            MEMORY_APPLY_PATCH => self.handle_memory_apply_patch(args).await,
            ARCHIVAL_MEMORY_INSERT => {
                memory::ArchivalMemoryInsertTool::run(&self.client, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            ARCHIVAL_MEMORY_SEARCH => {
                memory::ArchivalMemorySearchTool::run(&self.client, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            CONVERSATION_SEARCH => {
                memory::ConversationSearchTool::run(&self.client, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            SEARCH_MEMORY => memory::SearchMemoryTool::run(&self.client, &self.agent_id, args)
                .await
                .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false)),

            // -- Skill tools (intercepted; use local skill discovery)
            LOAD_SKILL => self.handle_load_skill(args),
            RUN_SKILL_SCRIPT => self.handle_run_skill_script(args).await,
            LOAD_SKILL_REF => self.handle_load_skill_ref(args),
            INSTALL_SKILL => self.handle_install_skill(args).await,

            // -- Checkpoints
            CREATE_CHECKPOINT => self.handle_create_checkpoint(args).await,
            LIST_CHECKPOINTS => self.handle_list_checkpoints().await,
            RESTORE_CHECKPOINT => self.handle_restore_checkpoint(args).await,

            // -- Artifacts
            STORE_ARTIFACT => self.handle_store_artifact(args).await,

            // -- Typed memory / provenance / reflection
            UPDATE_MEMORY_TYPED => self.handle_update_memory_typed(args).await,
            LINK_MEMORY_EVIDENCE => self.handle_link_memory_evidence(args).await,
            REFLECT => self.handle_reflect(args).await,

            // -- Code intelligence (Phase 3)
            SYMBOL_SEARCH => {
                crate::tools::codeintel::SymbolSearchTool::run(&self.client, &self.cwd, args)
                    .await
                    .map_or_else(|e| (e.to_string(), true), |o| (o, false))
            }
            FIND_REFERENCES => crate::tools::codeintel::FindReferencesTool::run(&self.client, args)
                .await
                .map_or_else(|e| (e.to_string(), true), |o| (o, false)),
            GOTO_DEFINITION => crate::tools::codeintel::GotoDefinitionTool::run(&self.client, args)
                .await
                .map_or_else(|e| (e.to_string(), true), |o| (o, false)),
            GET_REPO_MAP => {
                crate::tools::codeintel::GetRepoMapTool::run(&self.client, &self.cwd, args)
                    .await
                    .map_or_else(|e| (e.to_string(), true), |o| (o, false))
            }
            INDEX_REPOSITORY => crate::tools::codeintel::IndexRepositoryTool::run(
                &self.client,
                &self.agent_id,
                args,
            )
            .await
            .map_or_else(|e| (e.to_string(), true), |o| (o, false)),

            // -- Interactive tools — not handled here
            RUN_SUBAGENT | ASK_USER_QUESTION | ENTER_PLAN_MODE | EXIT_PLAN_MODE => {
                return None;
            }

            // -- Meta tools (agents)
            LIST_AGENTS => self.handle_list_agents().await,
            MESSAGE_AGENT => self.handle_message_agent(args).await,

            // -- Web tools (Phase 6)
            #[cfg(feature = "web")]
            WEB_SEARCH => cade_web::WebSearchTool::run(args)
                .await
                .map_or_else(|e| (e.to_string(), true), |o| (o, false)),
            #[cfg(feature = "web")]
            FETCH_DOC => cade_web::FetchDocTool::run(args)
                .await
                .map_or_else(|e| (e.to_string(), true), |o| (o, false)),
            #[cfg(feature = "desktop")]
            BROWSER_SCREENSHOT => crate::tools::desktop::DesktopCaptureTool::run(args)
                .await
                .map_or_else(|e| (e.to_string(), true), |o| (o, false)),

            // -- Bash + filesystem tools routed through execution backend
            BASH if !self.is_local_backend() => self.handle_bash_via_backend(args).await,
            READ_FILE if !self.is_local_backend() => self.handle_read_via_backend(args).await,
            WRITE_FILE if !self.is_local_backend() => self.handle_write_via_backend(args).await,

            // -- Everything else: native Rust tools + MCP
            _ => {
                let r = dispatch(tool_call_id.clone(), canonical, args, &self.mcp).await;
                (r.output, r.is_error)
            }
        };

        // Fire-and-forget tool execution logging
        if self.log_executions {
            let duration_ms = t0.elapsed().as_millis() as u64;
            self.client.log_tool_execution_spawn(
                self.agent_id.clone(),
                tool_name.to_string(),
                serde_json::to_string(args).unwrap_or_default(),
                if output.len() > 1024 {
                    format!("{}…", &output[..1024])
                } else {
                    output.clone()
                },
                is_error,
                duration_ms,
            );
        }

        Some(RuntimeToolResult {
            tool_call_id,
            tool_name: tool_name.to_string(),
            output,
            is_error,
        })
    }

    // endregion: --- Dispatch

    // region:    --- Memory handlers

    async fn handle_update_memory(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let value = args["value"].as_str().unwrap_or("").to_string();
        let operation = args["operation"].as_str().unwrap_or("set");
        let description = args["description"].as_str().map(String::from);

        if label.is_empty() {
            return ("Error: 'label' is required".to_string(), true);
        }

        let final_value = if operation == "append" {
            let existing = self
                .client
                .get_memory(&self.agent_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.label == label)
                .map(|b| b.value)
                .unwrap_or_default();
            if existing.is_empty() {
                value
            } else {
                format!("{existing}\n{value}")
            }
        } else {
            value
        };

        match self
            .client
            .upsert_memory(&self.agent_id, &label, &final_value, description.as_deref())
            .await
        {
            Ok(_) => (format!("Memory block '{label}' updated"), false),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("exceeds character limit") {
                    let limit = parse_limit_from_error(&err_str).unwrap_or(2_000);
                    let trimmed = auto_trim_to_limit(&final_value, limit);
                    let orig = final_value.chars().count();
                    let kept = trimmed.chars().count();
                    match self
                        .client
                        .upsert_memory(&self.agent_id, &label, &trimmed, description.as_deref())
                        .await
                    {
                        Ok(_) => (
                            format!(
                                "Memory block '{label}' updated (auto-trimmed from {orig} to {kept} chars to fit the {limit}-char limit)."
                            ),
                            false,
                        ),
                        Err(e2) => (format!("Failed after auto-trim: {e2}"), true),
                    }
                } else {
                    (format!("Failed: {err_str}"), true)
                }
            }
        }
    }

    async fn handle_memory_apply_patch(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let patch = args["patch"].as_str().unwrap_or("").to_string();
        let description = args["description"].as_str().map(String::from);

        if label.is_empty() || patch.is_empty() {
            return ("Error: 'label' and 'patch' are required".to_string(), true);
        }

        // Get current value
        let current = self
            .client
            .get_memory(&self.agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == label)
            .map(|b| b.value)
            .unwrap_or_default();

        // Apply unified diff patch
        match apply_unified_diff(&current, &patch) {
            Ok(new_value) => {
                match self
                    .client
                    .upsert_memory(&self.agent_id, &label, &new_value, description.as_deref())
                    .await
                {
                    Ok(_) => (
                        format!("Memory block '{label}' patched successfully"),
                        false,
                    ),
                    Err(e) => (format!("Failed to save patched memory: {e}"), true),
                }
            }
            Err(e) => (format!("Patch failed: {e}"), true),
        }
    }

    // endregion: --- Memory handlers

    // region:    --- Skill handlers

    fn handle_load_skill(&self, args: &Value) -> (String, bool) {
        let id = args["id"].as_str().unwrap_or("").trim().to_string();
        if id.is_empty() {
            return ("Error: 'id' is required".to_string(), true);
        }
        let skills = discover_all_skills(&self.cwd, Some(&self.agent_id), None);
        match skills.into_iter().find(|s| s.id == id) {
            Some(s) => (s.to_context_block(), false),
            None => (
                format!("Skill '{id}' not found. Use /skills to list available skills."),
                true,
            ),
        }
    }

    async fn handle_install_skill(&self, args: &Value) -> (String, bool) {
        let url = args["url"].as_str().unwrap_or("").trim().to_string();
        let scope = args["scope"].as_str().unwrap_or("project");
        if url.is_empty() {
            return ("Error: 'url' is required".to_string(), true);
        }
        let target_dir = if scope == "global" {
            dirs::home_dir()
                .map(|h| h.join(".cade").join("skills"))
                .unwrap_or_else(|| self.cwd.join(".skills"))
        } else {
            self.cwd.join(".skills")
        };
        match cade_core::skills::install_skill_from_url(&url, &target_dir).await {
            Ok(skill) => (
                format!(
                    "Skill '{}' installed as [{}] in {} scope. It is now available via load_skill(\"{}\").",
                    skill.name, skill.id, scope, skill.id
                ),
                false,
            ),
            Err(e) => (format!("Failed to install skill: {e}"), true),
        }
    }

    async fn handle_run_skill_script(&self, args: &Value) -> (String, bool) {
        let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let script = args["script"].as_str().unwrap_or("").trim().to_string();
        let script_args: Vec<String> = args["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if skill_id.is_empty() || script.is_empty() {
            return (
                "Error: 'skill_id' and 'script' are required".to_string(),
                true,
            );
        }

        let skills = discover_all_skills(&self.cwd, Some(&self.agent_id), None);
        let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
            return (format!("Skill '{skill_id}' not found"), true);
        };

        let Some(sk) = skill.scripts.iter().find(|s| s.name == script).cloned() else {
            let available: Vec<&str> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
            let list = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                format!("Script '{script}' not found in skill '{skill_id}'. Available: {list}"),
                true,
            );
        };

        let mut cmd = tokio::process::Command::new(&sk.path);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        match cmd.args(&script_args).output().await {
            Err(e) => (format!("Failed to run script: {e}"), true),
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };
                let is_err = !out.status.success();
                (combined, is_err)
            }
        }
    }

    // region:    --- Checkpoint handlers

    async fn handle_create_checkpoint(&self, args: &Value) -> (String, bool) {
        let label = args["label"]
            .as_str()
            .unwrap_or("checkpoint")
            .trim()
            .to_string();
        let description = args["description"].as_str().map(String::from);

        // 1. Attempt a git stash
        let git_cp = git_checkpoint::create_git_checkpoint(&label, &self.cwd).await;
        let stash_ref = git_cp
            .as_ref()
            .and_then(|g| g.stash_ref.as_deref())
            .map(String::from);
        let commit_hash = git_cp
            .as_ref()
            .and_then(|g| g.commit_hash.as_deref())
            .map(String::from);

        // 2. Create server-side checkpoint record
        let conv_id = self.conversation_id.as_deref();
        match self
            .client
            .create_checkpoint(
                &self.agent_id,
                Some(&label),
                description.as_deref(),
                conv_id,
                stash_ref.as_deref(),
                commit_hash.as_deref(),
            )
            .await
        {
            Ok(cp_id) => {
                let mut msg = format!("Checkpoint '{label}' created. ID: {cp_id}");
                if let Some(s) = &stash_ref {
                    msg.push_str(&format!("\nGit stash: {s}"));
                }
                if let Some(h) = &commit_hash {
                    msg.push_str(&format!("\nHEAD: {}", &h[..8.min(h.len())]));
                }
                (msg, false)
            }
            Err(e) => (format!("Failed to create checkpoint: {e}"), true),
        }
    }

    async fn handle_list_checkpoints(&self) -> (String, bool) {
        match self.client.list_checkpoints(&self.agent_id).await {
            Ok(list) if list.is_empty() => ("No checkpoints found.".to_string(), false),
            Ok(list) => {
                let mut out = format!("{} checkpoint(s):\n", list.len());
                for cp in &list {
                    let id = cp["id"].as_str().unwrap_or("?");
                    let label = cp["label"].as_str().unwrap_or("(unlabelled)");
                    let ts = cp["created_at"].as_i64().unwrap_or(0);
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .map(|d: chrono::DateTime<chrono::Utc>| {
                            d.format("%Y-%m-%d %H:%M").to_string()
                        })
                        .unwrap_or_default();
                    out.push_str(&format!("  {id}  [{label}]  {dt}\n"));
                }
                (out.trim_end().to_string(), false)
            }
            Err(e) => (format!("Failed to list checkpoints: {e}"), true),
        }
    }

    async fn handle_restore_checkpoint(&self, args: &Value) -> (String, bool) {
        let cp_id = args["checkpoint_id"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if cp_id.is_empty() {
            return ("Error: 'checkpoint_id' is required".to_string(), true);
        }

        // Get the checkpoint to find git info
        let cp = match self.client.get_checkpoint(&self.agent_id, &cp_id).await {
            Ok(v) => v,
            Err(e) => return (format!("Checkpoint not found: {e}"), true),
        };

        // Apply git stash if there is one
        let stash_ref = cp["git_stash_ref"].as_str().unwrap_or("").to_string();
        if !stash_ref.is_empty()
            && let Err(e) = git_checkpoint::restore_git_checkpoint(&stash_ref, &self.cwd).await
        {
            return (format!("Git restore failed: {e}"), true);
        }

        // Mark checkpoint as restored on server
        if let Err(e) = self.client.restore_checkpoint(&self.agent_id, &cp_id).await {
            tracing::warn!("restore_checkpoint server update failed: {e}");
        }

        let label = cp["label"].as_str().unwrap_or("?");
        (
            format!("Restored to checkpoint '{label}' ({cp_id})."),
            false,
        )
    }

    // endregion: --- Checkpoint handlers

    // region:    --- Artifact handlers

    async fn handle_store_artifact(&self, args: &Value) -> (String, bool) {
        let kind = args["kind"].as_str().unwrap_or("other");
        let content = args["content"].as_str().unwrap_or("");
        let label = args["label"].as_str().unwrap_or("");

        if content.is_empty() {
            return ("Error: 'content' is required".to_string(), true);
        }

        match self
            .client
            .store_artifact(
                &self.agent_id,
                kind,
                "text/plain",
                Some(content),
                None,
                None,
            )
            .await
        {
            Ok(art_id) => {
                let label_str = if label.is_empty() {
                    String::new()
                } else {
                    format!(" '{label}'")
                };
                (format!("Artifact{label_str} stored. ID: {art_id}"), false)
            }
            Err(e) => (format!("Failed to store artifact: {e}"), true),
        }
    }

    // endregion: --- Artifact handlers

    // region:    --- Backend helpers

    fn is_local_backend(&self) -> bool {
        self.backend.name() == "local"
    }

    async fn handle_bash_via_backend(&self, args: &Value) -> (String, bool) {
        let command = args["command"].as_str().unwrap_or("").to_string();
        let timeout_secs = args["timeout"].as_u64().unwrap_or(120);

        // Safety check even through non-local backends
        if !self.backend.is_writable() && cade_core::permissions::bash_command_is_write(&command) {
            return (
                format!(
                    "Blocked: read-only backend refuses write command: {}",
                    &command[..80.min(command.len())]
                ),
                true,
            );
        }

        match self
            .backend
            .exec_bash(&command, &self.cwd, timeout_secs)
            .await
        {
            Ok(out) => (out.combined(), out.exit_code != 0),
            Err(e) => (format!("Backend exec failed: {e}"), true),
        }
    }

    async fn handle_read_via_backend(&self, args: &Value) -> (String, bool) {
        let path_str = args["path"].as_str().unwrap_or("").trim().to_string();
        if path_str.is_empty() {
            return ("Error: 'path' is required".to_string(), true);
        }
        let path = std::path::Path::new(&path_str);
        match self.backend.read_file(path).await {
            Ok(content) => {
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(0) as usize;
                let lines: Vec<&str> = content.lines().collect();
                let total = lines.len();
                let end = if limit > 0 {
                    (offset + limit).min(total)
                } else {
                    total
                };
                let selected = &lines[offset.min(total)..end];
                let numbered: String = selected
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{:>4}→{}\n", offset + i + 1, l))
                    .collect();
                (format!("{numbered}[{total} lines total]"), false)
            }
            Err(e) => (format!("Read failed: {e}"), true),
        }
    }

    async fn handle_write_via_backend(&self, args: &Value) -> (String, bool) {
        if !self.backend.is_writable() {
            return ("Error: backend is read-only".to_string(), true);
        }
        let path_str = args["path"].as_str().unwrap_or("").trim().to_string();
        let content = args["content"].as_str().unwrap_or("").to_string();
        if path_str.is_empty() {
            return ("Error: 'path' is required".to_string(), true);
        }
        let path = std::path::Path::new(&path_str);
        match self.backend.write_file(path, &content).await {
            Ok(()) => (
                format!("Written {} bytes to {path_str}", content.len()),
                false,
            ),
            Err(e) => (format!("Write failed: {e}"), true),
        }
    }

    // endregion: --- Backend helpers

    // region:    --- Typed memory / provenance / reflection handlers

    async fn handle_update_memory_typed(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let value = args["value"].as_str().unwrap_or("").to_string();
        let memory_type = args["memory_type"].as_str().unwrap_or("generic");
        let confidence = args["confidence"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);
        let tags: Vec<String> = args["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if label.is_empty() || value.is_empty() {
            return ("Error: 'label' and 'value' are required".to_string(), true);
        }

        match self
            .client
            .upsert_typed_memory(
                &self.agent_id,
                &label,
                &value,
                memory_type,
                confidence,
                &tags,
                None,
            )
            .await
        {
            Ok(_) => (
                format!(
                    "Memory block '{label}' stored as [{memory_type}] (confidence: {:.0}%)",
                    confidence * 100.0
                ),
                false,
            ),
            Err(e) => (format!("Failed to store typed memory: {e}"), true),
        }
    }

    async fn handle_link_memory_evidence(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let kind = args["kind"].as_str().unwrap_or("user_assertion");
        let reference = args["reference"].as_str().unwrap_or("").trim().to_string();
        let excerpt = args["excerpt"].as_str().map(String::from);

        if label.is_empty() || reference.is_empty() {
            return (
                "Error: 'label' and 'reference' are required".to_string(),
                true,
            );
        }

        match self
            .client
            .add_memory_evidence(&self.agent_id, &label, kind, &reference, excerpt.as_deref())
            .await
        {
            Ok(_) => (
                format!("Evidence linked to '{label}': [{kind}] {reference}"),
                false,
            ),
            Err(e) => (format!("Failed to link evidence: {e}"), true),
        }
    }

    async fn handle_reflect(&self, args: &Value) -> (String, bool) {
        let focus = args["focus"].as_str().map(String::from);

        match self
            .client
            .trigger_reflect(&self.agent_id, focus.as_deref())
            .await
        {
            Ok(summary) => (format!("Reflection complete: {summary}"), false),
            Err(e) => (format!("Reflection failed: {e}"), true),
        }
    }

    // endregion: --- Typed memory / provenance / reflection handlers

    fn handle_load_skill_ref(&self, args: &Value) -> (String, bool) {
        let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let doc = args["doc"].as_str().unwrap_or("").trim().to_string();

        if skill_id.is_empty() || doc.is_empty() {
            return ("Error: 'skill_id' and 'doc' are required".to_string(), true);
        }

        let skills = discover_all_skills(&self.cwd, Some(&self.agent_id), None);
        let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
            return (format!("Skill '{skill_id}' not found"), true);
        };

        let Some(r) = skill
            .references
            .iter()
            .find(|r| {
                r.name == doc || r.path.file_name().and_then(|n| n.to_str()).unwrap_or("") == doc
            })
            .cloned()
        else {
            let available: Vec<&str> = skill.references.iter().map(|r| r.name.as_str()).collect();
            let list = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                format!("Reference '{doc}' not found in skill '{skill_id}'. Available: {list}"),
                true,
            );
        };

        match std::fs::read_to_string(&r.path) {
            Ok(content) => (
                format!("# Reference: {doc} (skill: {skill_id})\n\n{content}"),
                false,
            ),
            Err(e) => (format!("Failed to read reference '{doc}': {e}"), true),
        }
    }

    // endregion: --- Skill handlers

    // region:    --- Agent handlers

    async fn handle_list_agents(&self) -> (String, bool) {
        match self.client.list_agents().await {
            Ok(agents) => {
                if agents.is_empty() {
                    return ("No other agents found.".to_string(), false);
                }
                let mut out = String::from("Available agents:\n");
                for agent in agents {
                    let name = &agent.name;
                    let id = &agent.id;
                    let desc = agent.description.as_deref().unwrap_or("No description");
                    out.push_str(&format!("- {name} ({id}): {desc}\n"));
                }
                (out.trim().to_string(), false)
            }
            Err(e) => (format!("Failed to list agents: {e}"), true),
        }
    }

    async fn handle_message_agent(&self, args: &Value) -> (String, bool) {
        let target = args["target"].as_str().unwrap_or("").trim().to_string();
        let message = args["message"].as_str().unwrap_or("").to_string();

        if target.is_empty() || message.is_empty() {
            return (
                "Error: 'target' and 'message' are required".to_string(),
                true,
            );
        }

        // Try to resolve target to an agent ID
        let target_id = match self.client.list_agents().await {
            Ok(agents) => {
                if let Some(agent) = agents.iter().find(|a| a.id == target || a.name == target) {
                    agent.id.clone()
                } else {
                    return (format!("Error: Agent '{target}' not found"), true);
                }
            }
            Err(e) => return (format!("Failed to query agents: {e}"), true),
        };

        match self
            .client
            .stream_message(&target_id, &message, |_| {})
            .await
        {
            Ok(messages) => {
                // Ensure we get all tool outputs if it used tools
                let mut out = String::new();
                for msg in messages {
                    if let Some(text) = msg.assistant_text()
                        && !text.is_empty()
                    {
                        out.push_str(text);
                    }
                }
                if out.trim().is_empty() {
                    ("Target agent returned an empty response".to_string(), false)
                } else {
                    (out.trim().to_string(), false)
                }
            }
            Err(e) => (format!("Failed to message agent: {e}"), true),
        }
    }

    // endregion: --- Agent handlers
}

// endregion: --- ToolRuntime

// region:    --- Support

/// Trim `value` to at most `limit` chars, keeping the newest (tail) content.
pub fn auto_trim_to_limit(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        return value.to_string();
    }
    const NOTE: &str = "[...older content auto-trimmed to fit memory limit...]\n";
    let note_len = NOTE.chars().count();
    let keep = limit.saturating_sub(note_len);
    if keep == 0 {
        return value.chars().take(limit).collect();
    }
    let tail: String = value.chars().skip(count.saturating_sub(keep)).collect();
    format!("{NOTE}{tail}")
}

/// Extract the numeric upper limit from an "exceeds character limit (A > B)" error string.
pub fn parse_limit_from_error(error: &str) -> Option<usize> {
    let open = error.find('(')?;
    let close = error[open..].find(')')? + open;
    let inner = &error[open + 1..close];
    inner.split('>').nth(1)?.trim().parse().ok()
}

/// Apply a unified diff patch to `original` text.
/// This is a best-effort implementation suitable for memory block editing.
fn apply_unified_diff(original: &str, patch: &str) -> crate::Result<String> {
    // Simple line-based patch application.
    // For memory blocks (small text), this is sufficient.
    let orig_lines: Vec<&str> = original.lines().collect();
    let mut result: Vec<&str> = Vec::new();
    let mut orig_idx = 0usize;

    for line in patch.lines() {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
            // Parse hunk header to find position
            if let Some(hdr) = line.strip_prefix("@@")
                && let Some(hunk_start) = parse_hunk_start(hdr)
            {
                // Copy original lines up to the hunk start
                let target = hunk_start.saturating_sub(1);
                while orig_idx < target && orig_idx < orig_lines.len() {
                    result.push(orig_lines[orig_idx]);
                    orig_idx += 1;
                }
            }
        } else if let Some(add) = line.strip_prefix('+') {
            result.push(add);
        } else if let Some(_del) = line.strip_prefix('-') {
            // Skip the deleted line in original
            orig_idx += 1;
        } else if let Some(ctx) = line.strip_prefix(' ') {
            result.push(ctx);
            orig_idx += 1;
        }
    }

    // Append any remaining original lines
    while orig_idx < orig_lines.len() {
        result.push(orig_lines[orig_idx]);
        orig_idx += 1;
    }

    Ok(result.join("\n"))
}

fn parse_hunk_start(hdr: &str) -> Option<usize> {
    // Format: " -A,B +C,D @@"  — we want C (new file start line)
    let plus_part = hdr.split_whitespace().find(|s| s.starts_with('+'))?;
    let num = plus_part.trim_start_matches('+').split(',').next()?;
    num.parse().ok()
}

// endregion: --- Support

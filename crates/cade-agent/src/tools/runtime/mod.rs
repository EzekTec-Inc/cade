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

use cade_core::tool_ids::*;

use crate::backends::{ExecutionBackend, LocalBackend};
use crate::mcp::McpManager;
use crate::tools::memory as store_memory;

// region:    --- Types

/// Result of a single tool execution.
#[derive(Debug, Clone)]
pub struct RuntimeToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
    pub ui_resource_uri: Option<String>,
}

/// Executable capabilities supplied by a host without coupling native tools to packages.
#[async_trait::async_trait]
pub trait ToolExtension: Send + Sync {
    fn has_tool(&self, name: &str) -> bool;
    async fn execute(&self, call_id: &str, name: &str, args: &Value) -> RuntimeToolResult;
}

/// Fallback selection is captured before approvals/hooks and never re-resolved.
pub(crate) enum ToolBinding {
    Mcp { generation: String, is_write: bool },
    Remote { generation: String, is_write: bool },
    Extension(Arc<dyn ToolExtension>),
    Unavailable,
}

impl ToolBinding {
    pub(crate) fn is_write(&self, name: &str) -> bool {
        match self {
            Self::Mcp { is_write, .. } | Self::Remote { is_write, .. } => *is_write,
            Self::Extension(_) => true,
            Self::Unavailable => name.contains("__"),
        }
    }
}

// endregion: --- Types

// region:    --- ToolRuntime

pub mod agents;
pub mod checkpoints;
pub mod memory;
pub mod native;
pub mod skills;

/// Shared context for dispatching tool calls.
///
/// Create once per session and reuse across turns.
pub struct ToolRuntime {
    pub storage: Arc<dyn crate::backends::storage::StorageBackend>,
    pub mcp: Arc<McpManager>,
    pub agent_id: String,
    pub cwd: PathBuf,
    /// Active conversation ID — used for tool execution logging context.
    pub conversation_id: Option<String>,
    /// When true, each tool execution is logged to the server asynchronously.
    pub log_executions: bool,
    /// Pluggable execution backend (local / docker / ssh / readonly).
    pub backend: Arc<dyn ExecutionBackend>,
    /// Restrict file I/O tools to these paths. Only paths starting with one of these prefixes are allowed.
    pub allowed_paths: Option<Vec<String>>,
    pub extension: Option<Arc<dyn ToolExtension>>,
}

impl ToolRuntime {
    // -- Constructor

    pub fn new(
        storage: Arc<dyn crate::backends::storage::StorageBackend>,
        mcp: Arc<McpManager>,
        agent_id: String,
        cwd: PathBuf,
    ) -> Self {
        Self {
            storage,
            mcp,
            agent_id,
            cwd,
            conversation_id: None,
            log_executions: false,
            backend: Arc::new(LocalBackend),
            allowed_paths: None,
            extension: None,
        }
    }

    /// Convenience constructor that wraps an MCP reference.
    pub fn from_refs(
        storage: Arc<dyn crate::backends::storage::StorageBackend>,
        mcp: Arc<McpManager>,
        agent_id: &str,
        cwd: PathBuf,
    ) -> Self {
        Self {
            storage,
            mcp,
            agent_id: agent_id.to_string(),
            cwd,
            conversation_id: None,
            log_executions: false,
            backend: Arc::new(LocalBackend),
            allowed_paths: None,
            extension: None,
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

    pub fn with_extension(mut self, extension: Arc<dyn ToolExtension>) -> Self {
        self.extension = Some(extension);
        self
    }

    /// The arguments authorized, observed and executed must name the same workspace paths.
    pub fn prepare_arguments(&self, name: &str, args: &Value) -> Value {
        let canonical = crate::tools::manager::canonical_name(name);
        let mut args = args.clone();
        if matches!(canonical, "glob" | "grep") && args.get("path").is_none() {
            args["path"] = Value::String(self.cwd.to_string_lossy().into_owned());
        }
        crate::tools::normalize_mcp_arguments(canonical, &args, &self.cwd)
    }

    pub fn extension_is_write(&self, name: &str) -> bool {
        self.extension
            .as_ref()
            .is_some_and(|extension| extension.has_tool(name))
    }

    pub(crate) async fn bind_tool(&self, name: &str) -> ToolBinding {
        if name.contains("__")
            && let Some((generation, is_write)) = self.mcp.tool_binding(name).await
        {
            return ToolBinding::Mcp {
                generation,
                is_write,
            };
        }
        if let Some(extension) = &self.extension
            && extension.has_tool(name)
        {
            return ToolBinding::Extension(extension.clone());
        }
        if name.contains("__")
            && let Ok(Some((generation, is_write))) = self.storage.mcp_tool_binding(name).await
        {
            return ToolBinding::Remote {
                generation,
                is_write,
            };
        }
        ToolBinding::Unavailable
    }

    /// Access the working directory.
    pub fn working_dir(&self) -> &std::path::Path {
        &self.cwd
    }

    /// Access the MCP manager reference.
    pub fn mcp(&self) -> &Arc<McpManager> {
        &self.mcp
    }

    /// Access the agent ID.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
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
        let args = self.prepare_arguments(tool_name, args);
        self.execute_prepared(tool_call_id, tool_name, &args).await
    }

    /// Dispatch prepared arguments. ToolPipeline uses execute_bound to retain its
    /// pre-authorization implementation snapshot across approvals and hooks.
    pub(crate) async fn execute_prepared(
        &self,
        tool_call_id: String,
        tool_name: &str,
        args: &Value,
    ) -> Option<RuntimeToolResult> {
        let binding = self.bind_tool(tool_name).await;
        self.execute_bound(tool_call_id, tool_name, args, &binding)
            .await
    }

    pub(crate) async fn execute_bound(
        &self,
        tool_call_id: String,
        tool_name: &str,
        args: &Value,
        binding: &ToolBinding,
    ) -> Option<RuntimeToolResult> {
        crate::tools::fs::in_workspace(
            &self.cwd,
            self.execute_in_workspace(tool_call_id, tool_name, args, binding),
        )
        .await
    }

    async fn execute_in_workspace(
        &self,
        tool_call_id: String,
        tool_name: &str,
        args: &Value,
        binding: &ToolBinding,
    ) -> Option<RuntimeToolResult> {
        // Normalise Gemini / Codex aliases back to canonical IDs.
        let canonical_owned: String = {
            use cade_core::toolsets::Toolset;
            use cade_core::toolsets::adapter::ToolSurfaceAdapter;
            let ga = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);
            ga.to_canonical(tool_name).to_string()
        };
        let canonical = canonical_owned.as_str();

        if let Err(error) = crate::tools::fs::check_path_grants(
            canonical,
            args,
            self.allowed_paths.as_deref(),
            &self.cwd,
        ) {
            return Some(RuntimeToolResult {
                tool_call_id,
                tool_name: tool_name.into(),
                output: error.to_string(),
                is_error: true,
                ui_resource_uri: None,
            });
        }

        let mut ui_resource_uri = None;

        let t0 = std::time::Instant::now();
        let (output, is_error): (String, bool) = match canonical {
            // -- Memory tools (intercepted; use REST client)
            UPDATE_MEMORY => self.handle_update_memory(args).await,
            MEMORY_APPLY_PATCH => self.handle_memory_apply_patch(args).await,
            ARCHIVAL_MEMORY_INSERT => {
                store_memory::ArchivalMemoryInsertTool::run(&*self.storage, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            ARCHIVAL_MEMORY_SEARCH => {
                store_memory::ArchivalMemorySearchTool::run(&*self.storage, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            CONVERSATION_SEARCH => {
                store_memory::ConversationSearchTool::run(&*self.storage, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }
            SEARCH_MEMORY => {
                store_memory::SearchMemoryTool::run(&*self.storage, &self.agent_id, args)
                    .await
                    .map_or_else(|e| (format!("Failed: {e}"), false), |o| (o, false))
            }

            // -- Skill tools (intercepted; use local skill discovery)
            LOAD_SKILL => self.handle_load_skill(args),
            RUN_SKILL_SCRIPT => self.handle_run_skill_script(args).await,
            LOAD_SKILL_REF => self.handle_load_skill_ref(args),
            INSTALL_SKILL => self.handle_install_skill(args).await,
            INSTALL_PLUGIN => self.handle_install_plugin(args).await,

            // -- Checkpoints
            CREATE_CHECKPOINT => self.handle_create_checkpoint(args).await,
            LIST_CHECKPOINTS => self.handle_list_checkpoints().await,
            RESTORE_CHECKPOINT => self.handle_restore_checkpoint(args).await,

            // -- Artifacts
            STORE_ARTIFACT => self.handle_store_artifact(args).await,

            // -- Typed memory / provenance / reflection
            UPDATE_MEMORY_TYPED => self.handle_update_memory_typed(args).await,
            UPDATE_MEMORY_FIELD => self.handle_update_memory_field(args).await,
            LINK_MEMORY_EVIDENCE => self.handle_link_memory_evidence(args).await,
            REFLECT => self.handle_reflect(args).await,
            RECALL => self.handle_recall(args).await,
            ANSWER => self.handle_answer(args).await,

            // -- Interactive tools — not handled here
            RUN_SUBAGENT | ASK_USER_QUESTION | ENTER_PLAN_MODE | EXIT_PLAN_MODE => {
                return None;
            }

            // -- Meta tools (agents)
            LIST_AGENTS => self.handle_list_agents().await,
            MESSAGE_AGENT => self.handle_message_agent(args).await,
            SEARCH_TOOLS => self.handle_search_tools(args).await,

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
            EDIT_FILE if !self.is_local_backend() => self.handle_edit_via_backend(args).await,
            APPLY_PATCH | GREP | GLOB if !self.is_local_backend() => (
                format!(
                    "Tool '{canonical}' is not supported by execution backend '{}'; host execution refused",
                    self.backend.name()
                ),
                true,
            ),

            // -- Everything else: native Rust tools + MCP (local or remote server)
            _ => {
                let r = crate::tools::manager::dispatch_prepared(
                    tool_call_id.clone(),
                    canonical,
                    args,
                    &self.mcp,
                    self.allowed_paths.as_deref(),
                    match binding {
                        ToolBinding::Mcp { generation, .. } => Some(generation.as_str()),
                        _ => None,
                    },
                )
                .await;
                if r.is_error && r.output.starts_with("Unknown tool:") {
                    if let ToolBinding::Extension(extension) = binding {
                        if !self.is_local_backend() || !self.backend.is_writable() {
                            return Some(RuntimeToolResult {
                                tool_call_id,
                                tool_name: tool_name.to_owned(),
                                output: "Native plugin execution requires a writable local backend"
                                    .into(),
                                is_error: true,
                                ui_resource_uri: None,
                            });
                        }
                        return Some(extension.execute(&tool_call_id, canonical, args).await);
                    }
                    // Try remote server-hosted MCP
                    let remote = match binding {
                        ToolBinding::Remote { generation, .. } => {
                            self.storage
                                .call_mcp_tool_bound(canonical, args, generation)
                                .await
                        }
                        _ => Err(crate::Error::custom(r.output.clone())),
                    };
                    match remote {
                        Ok((out, err_flag, uri)) => {
                            ui_resource_uri = uri;
                            (out, err_flag)
                        }
                        Err(error) => {
                            ui_resource_uri = r.ui_resource_uri;
                            (error.to_string(), true)
                        }
                    }
                } else {
                    ui_resource_uri = r.ui_resource_uri;
                    (r.output, r.is_error)
                }
            }
        };

        // Fire-and-forget tool execution logging
        if self.log_executions {
            let duration_ms = t0.elapsed().as_millis() as u64;
            self.storage
                .log_tool_execution_spawn(
                    self.agent_id.clone(),
                    self.conversation_id.clone(),
                    None, // checkpoint ID not easily available here
                    tool_call_id.clone(),
                    tool_name.to_string(),
                    args.clone(),
                    if output.len() > 1024 {
                        format!("{}…", output.chars().take(1024).collect::<String>())
                    } else {
                        output.clone()
                    },
                    is_error,
                    duration_ms,
                )
                .await;
        }

        Some(RuntimeToolResult {
            tool_call_id,
            tool_name: tool_name.to_string(),
            output,
            is_error,
            ui_resource_uri,
        })
    }

    // endregion: --- Dispatch

    // region:    --- Skill handlers

    fn handle_load_skill(&self, _args: &Value) -> (String, bool) {
        (
            "load_skill is deprecated and removed from your schema. Please use the standard `read` tool to read the skill's `SKILL.md` file from the path provided in your system prompt instead.".to_string(),
            true,
        )
    }

    // region:    --- Checkpoint handlers

    async fn handle_list_checkpoints(&self) -> (String, bool) {
        match self.storage.list_checkpoints(&self.agent_id).await {
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

    // endregion: --- Checkpoint handlers

    // region:    --- Artifact handlers

    // endregion: --- Artifact handlers

    // region:    --- Backend helpers

    fn is_local_backend(&self) -> bool {
        self.backend.name() == "local"
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
        let _lock = crate::tools::file_lock::FileLockManager::global()
            .acquire_lock(path)
            .await;
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

    async fn handle_reflect(&self, args: &Value) -> (String, bool) {
        let focus = args["focus"].as_str().map(String::from);

        match self
            .storage
            .trigger_reflect(&self.agent_id, focus.as_deref())
            .await
        {
            Ok(_) => ("Reflection triggered".to_string(), false),
            Err(e) => (format!("Reflection failed: {e}"), true),
        }
    }

    // endregion: --- Typed memory / provenance / reflection handlers

    // endregion: --- Skill handlers

    // region:    --- Agent handlers

    async fn handle_list_agents(&self) -> (String, bool) {
        match self.storage.list_agents().await {
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
/// Apply a unified-diff patch to a string.
///
/// Public for re-use by cade-server's server-side meta-tool intercepts
/// (Phase A1: `memory_apply_patch`).  Internal implementation detail —
/// not part of the stable cade-agent API.
pub fn apply_unified_diff(original: &str, patch: &str) -> crate::Result<String> {
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    /// Build a ToolRuntime with a fake client (no actual HTTP).
    fn build_test_runtime() -> ToolRuntime {
        let client = Arc::new(
            crate::agent::HttpTransport::new(
                "http://localhost:0".to_string(),
                "fake-key".to_string(),
            )
            .unwrap(),
        );
        let mcp = Arc::new(crate::mcp::McpManager::empty());
        ToolRuntime::new(client, mcp, "test-agent".to_string(), PathBuf::from("/tmp"))
    }

    struct DelayedFileBackend {
        files: Mutex<BTreeMap<PathBuf, String>>,
    }

    #[async_trait::async_trait]
    impl ExecutionBackend for DelayedFileBackend {
        async fn exec_bash(
            &self,
            _command: &str,
            _cwd: &Path,
            _timeout_secs: u64,
        ) -> crate::Result<crate::backends::BashOutput> {
            Err(crate::Error::custom("shell execution is not supported"))
        }

        async fn read_file(&self, path: &Path) -> crate::Result<String> {
            let snapshot = self
                .files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| crate::Error::custom("file not found"))?;
            // Yield after taking the snapshot: without runtime serialization,
            // concurrent edits both read the original content before either writes.
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            Ok(snapshot)
        }

        async fn write_file(&self, path: &Path, content: &str) -> crate::Result<()> {
            tokio::time::sleep(Duration::from_millis(25)).await;
            self.files
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), content.to_string());
            Ok(())
        }

        async fn path_exists(&self, path: &Path) -> bool {
            self.files.lock().unwrap().contains_key(path)
        }

        async fn list_dir(&self, _path: &Path) -> crate::Result<Vec<crate::backends::DirEntry>> {
            Err(crate::Error::custom("directory listing is not supported"))
        }

        fn is_writable(&self) -> bool {
            true
        }

        fn name(&self) -> &'static str {
            "delayed-test-backend"
        }
    }

    fn build_backend_test_runtime(cwd: PathBuf, backend: Arc<DelayedFileBackend>) -> ToolRuntime {
        ToolRuntime::new(
            Arc::new(MockRemoteMcpStorage {
                expected_name: String::new(),
                return_output: String::new(),
            }),
            Arc::new(crate::mcp::McpManager::empty()),
            "test-agent".to_string(),
            cwd,
        )
        .with_backend(backend)
    }

    #[tokio::test]
    async fn backend_edits_preserve_concurrent_disjoint_changes() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("shared.txt");
        let backend = Arc::new(DelayedFileBackend {
            files: Mutex::new(BTreeMap::from([(
                path.clone(),
                "alpha=old\nbeta=old\n".to_string(),
            )])),
        });
        let mut rt = build_backend_test_runtime(workspace.path().to_path_buf(), backend.clone());
        rt.allowed_paths = Some(vec![workspace.path().to_string_lossy().into_owned()]);
        let alpha = serde_json::json!({
            "path": "shared.txt", "old_string": "alpha=old", "new_string": "alpha=new"
        });
        let beta = serde_json::json!({
            "path": "shared.txt", "old_string": "beta=old", "new_string": "beta=new"
        });

        let (alpha_result, beta_result) = tokio::join!(
            rt.execute("edit-alpha".into(), "edit_file", &alpha),
            rt.execute("edit-beta".into(), "edit_file", &beta),
        );

        for result in [alpha_result, beta_result] {
            let result = result.expect("edit_file must be handled by ToolRuntime");
            assert!(!result.is_error, "{}", result.output);
        }
        assert_eq!(
            backend.files.lock().unwrap().get(&path).unwrap(),
            "alpha=new\nbeta=new\n",
            "both successful edits must survive in the backend file"
        );
    }

    #[tokio::test]
    async fn backend_edits_respect_allowed_roots() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("allowed");
        let allowed_path = root.join("file.txt");
        let escaped_path = workspace.path().join("allowed-other/file.txt");
        let backend = Arc::new(DelayedFileBackend {
            files: Mutex::new(BTreeMap::from([
                (allowed_path.clone(), "original".to_string()),
                (escaped_path.clone(), "original".to_string()),
            ])),
        });
        let mut rt = build_backend_test_runtime(workspace.path().to_path_buf(), backend.clone());
        rt.allowed_paths = Some(vec![root.to_string_lossy().into_owned()]);

        for path in [escaped_path.clone(), root.join("../allowed-other/file.txt")] {
            for tool in ["edit_file", "write_file"] {
                let result = rt
                    .execute(
                        format!("denied-{tool}"),
                        tool,
                        &serde_json::json!({
                            "path": path,
                            "old_string": "original",
                            "new_string": "changed",
                            "content": "changed"
                        }),
                    )
                    .await
                    .unwrap();
                assert!(result.is_error, "{tool} must reject {}", path.display());
                assert!(
                    result.output.contains("[Blocked by RBAC]"),
                    "{}",
                    result.output
                );
            }
        }

        let result = rt
            .execute(
                "allowed-edit".into(),
                "edit_file",
                &serde_json::json!({
                    "path": "allowed/file.txt",
                    "old_string": "original",
                    "new_string": "changed"
                }),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            *backend.files.lock().unwrap(),
            BTreeMap::from([
                (allowed_path, "changed".to_string()),
                (escaped_path, "original".to_string()),
            ])
        );
    }

    // -- Bug 7: ToolRuntime returns None for interactive-only tools

    #[tokio::test]
    async fn runtime_returns_none_for_run_subagent() {
        let rt = build_test_runtime();
        let result = rt
            .execute(
                "tc_1".into(),
                "run_subagent",
                &serde_json::json!({"prompt": "hello"}),
            )
            .await;
        assert!(
            result.is_none(),
            "run_subagent must not be handled by ToolRuntime"
        );
    }

    #[tokio::test]
    async fn runtime_returns_none_for_ask_user_question() {
        let rt = build_test_runtime();
        let result = rt
            .execute("tc_2".into(), "ask_user_question", &serde_json::json!({}))
            .await;
        assert!(
            result.is_none(),
            "ask_user_question must not be handled by ToolRuntime"
        );
    }

    #[tokio::test]
    async fn runtime_returns_none_for_enter_plan_mode() {
        let rt = build_test_runtime();
        let result = rt
            .execute("tc_3".into(), "EnterPlanMode", &serde_json::json!({}))
            .await;
        assert!(result.is_none());
    }

    // Verify that a known tool DOES return Some

    #[tokio::test]
    async fn runtime_returns_some_for_read_file() {
        let rt = build_test_runtime();
        let result = rt
            .execute(
                "tc_4".into(),
                "read_file",
                &serde_json::json!({"path": "/dev/null"}),
            )
            .await;
        assert!(result.is_some(), "read_file must be handled by ToolRuntime");
    }

    #[tokio::test]
    async fn runtime_executes_search_tools_empty() {
        let rt = build_test_runtime();
        let result = rt
            .execute(
                "tc_5".into(),
                "search_tools",
                &serde_json::json!({"query": "postgres"}),
            )
            .await;
        assert!(result.is_some());
        let res = result.unwrap();
        assert!(!res.is_error);
        assert!(
            res.output
                .contains("No third-party MCP tools matched search query 'postgres'")
        );
    }

    struct MockRemoteMcpStorage {
        expected_name: String,
        return_output: String,
    }

    #[async_trait::async_trait]
    impl crate::backends::storage::StorageBackend for MockRemoteMcpStorage {
        async fn get_memory(
            &self,
            _agent_id: &str,
        ) -> crate::Result<Vec<crate::agent::client::MemoryBlock>> {
            Ok(vec![])
        }
        async fn delete_memory(&self, _agent_id: &str, _label: &str) -> crate::Result<()> {
            Ok(())
        }
        async fn upsert_memory_with_limit(
            &self,
            _agent_id: &str,
            _label: &str,
            _value: &str,
            _desc: Option<&str>,
            _limit: Option<usize>,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn upsert_memory_with_options(
            &self,
            _agent_id: &str,
            _label: &str,
            _value: &str,
            _desc: Option<&str>,
            _limit: Option<usize>,
            _memory_type: Option<&str>,
            _confidence: Option<f64>,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn search_memory(
            &self,
            _agent_id: &str,
            _query: &str,
            _memory_type: Option<&str>,
        ) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn conversation_search(
            &self,
            _agent_id: &str,
            _keyword: &str,
            _limit: Option<usize>,
        ) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn archival_memory_insert(
            &self,
            _agent_id: &str,
            _content: &str,
            _tags: Option<&[String]>,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn archival_memory_search(
            &self,
            _agent_id: &str,
            _keyword: &str,
            _limit: Option<usize>,
        ) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn query_event_log(
            &self,
            _agent_id: &str,
            _keyword: &str,
            _limit: Option<usize>,
        ) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn recall(
            &self,
            _agent_id: &str,
            _query: &str,
            _limit: Option<usize>,
        ) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn add_memory_evidence(
            &self,
            _agent_id: &str,
            _label: &str,
            _kind: &str,
            _reference: &str,
            _excerpt: Option<&str>,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn trigger_reflect(
            &self,
            _agent_id: &str,
            _focus: Option<&str>,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn record_recent_edit(&self, _agent_id: &str, _path: &str) -> crate::Result<()> {
            Ok(())
        }
        async fn store_artifact(
            &self,
            _agent_id: &str,
            _kind: &str,
            _content_type: &str,
            _text: Option<&str>,
            _blob: Option<&[u8]>,
            _metadata: Option<&serde_json::Value>,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn install_plugin(
            &self,
            _agent_id: &str,
            _url: &str,
            _plugin_id: &str,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn install_skill(
            &self,
            _agent_id: &str,
            _url: &str,
            _scope: &str,
            _skill_name: Option<&str>,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn run_skill_script(
            &self,
            _agent_id: &str,
            _skill_id: &str,
            _script_name: &str,
            _args: Option<&[String]>,
            _cwd: &std::path::Path,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn load_skill_ref(
            &self,
            _agent_id: &str,
            _skill_id: &str,
            _doc_name: &str,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn create_checkpoint(
            &self,
            _agent_id: &str,
            _conversation_id: Option<&str>,
            _branch_id: Option<&str>,
            _label: Option<&str>,
            _desc: Option<&str>,
            _git_commit_hash: Option<&str>,
        ) -> crate::Result<String> {
            Ok(String::new())
        }
        async fn get_checkpoint(
            &self,
            _agent_id: &str,
            _checkpoint_id: &str,
        ) -> crate::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
        async fn list_checkpoints(&self, _agent_id: &str) -> crate::Result<Vec<serde_json::Value>> {
            Ok(vec![])
        }
        async fn restore_checkpoint(
            &self,
            _agent_id: &str,
            _checkpoint_id: &str,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn list_agents(&self) -> crate::Result<Vec<crate::agent::client::AgentState>> {
            Ok(vec![crate::agent::client::AgentState {
                id: "helper-id".into(),
                name: "helper".into(),
                model: None,
                description: None,
                system_prompt: None,
            }])
        }
        async fn message_agent(
            &self,
            _agent_id: &str,
            target: &str,
            message: &str,
        ) -> crate::Result<String> {
            assert_eq!(target, "helper-id");
            assert_eq!(message, "hello");
            Ok("received".into())
        }
        async fn log_tool_execution_spawn(
            &self,
            _agent_id: String,
            _conversation_id: Option<String>,
            _checkpoint_id: Option<String>,
            _tool_call_id: String,
            _tool_name: String,
            _arguments: serde_json::Value,
            _output: String,
            _is_error: bool,
            _duration_ms: u64,
        ) {
        }
        async fn stamp_provenance(
            &self,
            _agent_id: &str,
            _label: &str,
            _tool_call_id: Option<&str>,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn mcp_tool_binding(&self, name: &str) -> crate::Result<Option<(String, bool)>> {
            Ok((name == self.expected_name).then(|| ("fixture".into(), false)))
        }
        async fn call_mcp_tool_bound(
            &self,
            name: &str,
            arguments: &Value,
            generation: &str,
        ) -> crate::Result<(String, bool, Option<String>)> {
            assert_eq!(generation, "fixture");
            self.call_mcp_tool(name, arguments).await
        }
        async fn call_mcp_tool(
            &self,
            name: &str,
            _arguments: &serde_json::Value,
        ) -> crate::Result<(String, bool, Option<String>)> {
            if name == self.expected_name {
                Ok((
                    self.return_output.clone(),
                    false,
                    Some("ui://test".to_string()),
                ))
            } else {
                Err(crate::Error::custom("tool not found"))
            }
        }
    }

    #[tokio::test]
    async fn runtime_dispatches_to_remote_mcp_storage() {
        let mock_storage = Arc::new(MockRemoteMcpStorage {
            expected_name: "serena__find_symbol".to_string(),
            return_output: "Symbol found: fn main()".to_string(),
        });
        let mcp = Arc::new(crate::mcp::McpManager::empty());
        let rt = ToolRuntime::new(
            mock_storage,
            mcp,
            "test-agent".to_string(),
            PathBuf::from("/tmp"),
        );

        let result = rt
            .execute(
                "call_mcp_1".into(),
                "serena__find_symbol",
                &serde_json::json!({"name_path_pattern": "main"}),
            )
            .await;
        assert!(result.is_some());
        let res = result.unwrap();
        assert!(!res.is_error);
        assert_eq!(res.output, "Symbol found: fn main()");
        assert_eq!(res.ui_resource_uri.as_deref(), Some("ui://test"));
    }

    #[tokio::test]
    async fn runtime_message_agent_preserves_target_through_preparation_and_dispatch() {
        let workspace = tempfile::tempdir().unwrap();
        let rt = ToolRuntime::new(
            Arc::new(MockRemoteMcpStorage {
                expected_name: String::new(),
                return_output: String::new(),
            }),
            Arc::new(crate::mcp::McpManager::empty()),
            "parent".into(),
            workspace.path().into(),
        );
        let result = rt
            .execute(
                "message".into(),
                "message_agent",
                &serde_json::json!({"target": "helper", "message": "hello"}),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(result.output, "received");
    }
}

// endregion: --- Tests

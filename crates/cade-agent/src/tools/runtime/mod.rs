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


use crate::backends::{ExecutionBackend, LocalBackend};
use crate::mcp::McpManager;
use crate::tools::dispatch;
use crate::tools::memory as store_memory;

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
        }
    }

    /// Convenience constructor that clones the client and wraps an MCP reference.
    pub fn from_refs(
        storage: Arc<dyn crate::backends::storage::StorageBackend>,
        mcp: &McpManager,
        agent_id: &str,
        cwd: PathBuf,
    ) -> Self {
        let _ = mcp;
        Self {
            storage,
            mcp: Arc::new(McpManager::empty()),
            agent_id: agent_id.to_string(),
            cwd,
            conversation_id: None,
            log_executions: false,
            backend: Arc::new(LocalBackend),
            allowed_paths: None,
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
                let r = dispatch(
                    tool_call_id.clone(),
                    canonical,
                    args,
                    &self.mcp,
                    self.allowed_paths.as_deref(),
                )
                .await;
                (r.output, r.is_error)
            }
        };

        // Fire-and-forget tool execution logging
        if self.log_executions {
            let duration_ms = t0.elapsed().as_millis() as u64;
            self.storage.log_tool_execution_spawn(
                self.agent_id.clone(),
                self.conversation_id.clone(),
                None, // checkpoint ID not easily available here
                tool_call_id.clone(),
                tool_name.to_string(),
                args.clone(),
                if output.len() > 1024 {
                    format!("{}…", &output[..1024])
                } else {
                    output.clone()
                },
                is_error,
                duration_ms,
            ).await;
        }

        Some(RuntimeToolResult {
            tool_call_id,
            tool_name: tool_name.to_string(),
            output,
            is_error,
        })
    }

    // endregion: --- Dispatch

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

        match self.storage
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
    use std::path::PathBuf;

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
}

// endregion: --- Tests

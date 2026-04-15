/// Hook engine — runs user-defined scripts at key lifecycle events.
///
/// Mirrors CADE Code's hook system:
///   - Events: PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest,
///     UserPromptSubmit, Stop, SubagentStop, SessionStart, SessionEnd, Notification
///   - Hook types: command (shell script via stdin JSON)
///   - Exit codes: 0=allow, 1=log+continue, 2=block+stderr→agent
///   - PostToolUse stdout with {"additionalContext":"..."} is injected into tool result
// region:    --- Modules
use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::{Value, json};

use crate::settings::HooksConfig;

// endregion: --- Modules

// -- Public outcome

#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Proceed normally
    Allow,
    /// Block the action; `reason` is fed back to the agent (or logged)
    Block { reason: String },
}

impl HookOutcome {
    pub fn is_block(&self) -> bool {
        matches!(self, Self::Block { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Block { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

// -- HookEngine

#[derive(Clone)]
pub struct HookEngine {
    hooks: Arc<HooksConfig>,
    cwd: PathBuf,
}

impl HookEngine {
    pub fn new(hooks: HooksConfig, cwd: PathBuf) -> Self {
        Self {
            hooks: Arc::new(hooks),
            cwd,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    // -- Tool lifecycle

    /// Fire before a tool runs. Returns `Block` to prevent execution.
    pub async fn pre_tool_use(&self, tool_name: &str, args: &Value) -> HookOutcome {
        let input = json!({
            "event_type":        "PreToolUse",
            "working_directory": self.cwd,
            "tool_name":         tool_name,
            "tool_input":        args,
        });
        self.run_entries_blocking(&self.hooks.pre_tool_use, tool_name, input)
            .await
    }

    /// Fire after a tool succeeds.
    /// Returns optional `additionalContext` string to append to the tool result.
    pub async fn post_tool_use(
        &self,
        tool_name: &str,
        args: &Value,
        output: &str,
        preceding_reasoning: Option<&str>,
        preceding_assistant_message: Option<&str>,
    ) -> Option<String> {
        let input = json!({
            "event_type":                "PostToolUse",
            "working_directory":         self.cwd,
            "tool_name":                 tool_name,
            "tool_input":                args,
            "tool_output":               output,
            "preceding_reasoning":       preceding_reasoning.unwrap_or(""),
            "preceding_assistant_message": preceding_assistant_message.unwrap_or(""),
        });
        self.run_entries_context(&self.hooks.post_tool_use, tool_name, input)
            .await
    }

    /// Fire after a tool fails (non-blocking).
    pub async fn post_tool_use_failure(
        &self,
        tool_name: &str,
        args: &Value,
        error: &str,
        preceding_reasoning: Option<&str>,
        preceding_assistant_message: Option<&str>,
    ) {
        let input = json!({
            "event_type":                "PostToolUseFailure",
            "working_directory":         self.cwd,
            "tool_name":                 tool_name,
            "tool_input":                args,
            "error":                     error,
            "preceding_reasoning":       preceding_reasoning.unwrap_or(""),
            "preceding_assistant_message": preceding_assistant_message.unwrap_or(""),
        });
        self.run_entries_fire_forget(&self.hooks.post_tool_use_failure, tool_name, input)
            .await;
    }

    /// Fire when the permission prompt is about to appear. Returns `Block` to
    /// suppress the prompt and deny the tool call outright.
    pub async fn permission_request(&self, tool_name: &str, args: &Value) -> HookOutcome {
        let input = json!({
            "event_type":        "PermissionRequest",
            "working_directory": self.cwd,
            "tool_name":         tool_name,
            "tool_input":        args,
        });
        self.run_entries_blocking(&self.hooks.permission_request, tool_name, input)
            .await
    }

    // -- Conversation lifecycle

    /// Fire when the user submits a prompt. Returns `Block` to suppress the turn.
    pub async fn user_prompt_submit(&self, prompt: &str) -> HookOutcome {
        let input = json!({
            "event_type":        "UserPromptSubmit",
            "working_directory": self.cwd,
            "prompt":            prompt,
        });
        self.run_all_blocking(&self.hooks.user_prompt_submit, input)
            .await
    }

    /// Fire when the agent finishes responding (no more tool calls). Returns
    /// `Block` to continue the agent (stderr is fed back as a new message).
    pub async fn stop(
        &self,
        stop_reason: &str,
        user_message: &str,
        assistant_message: &str,
        preceding_reasoning: Option<&str>,
    ) -> HookOutcome {
        let input = json!({
            "event_type":          "Stop",
            "working_directory":   self.cwd,
            "stop_reason":         stop_reason,
            "user_message":        user_message,
            "assistant_message":   assistant_message,
            "preceding_reasoning": preceding_reasoning.unwrap_or(""),
        });
        self.run_all_blocking(&self.hooks.stop, input).await
    }

    /// Fire when a subagent task completes.
    pub async fn subagent_stop(
        &self,
        subagent_type: &str,
        result: &str,
        is_error: bool,
    ) -> HookOutcome {
        let input = json!({
            "event_type":     "SubagentStop",
            "working_directory": self.cwd,
            "subagent_type":  subagent_type,
            "result":         result,
            "is_error":       is_error,
        });
        self.run_all_blocking(&self.hooks.subagent_stop, input)
            .await
    }

    /// Fire at session start (non-blocking).
    pub async fn session_start(&self, agent_id: &str) {
        let input = json!({
            "event_type":        "SessionStart",
            "working_directory": self.cwd,
            "agent_id":          agent_id,
        });
        self.run_all_fire_forget(&self.hooks.session_start, input)
            .await;
    }

    /// Fire at session end (non-blocking).
    pub async fn session_end(&self, agent_id: &str) {
        let input = json!({
            "event_type":        "SessionEnd",
            "working_directory": self.cwd,
            "agent_id":          agent_id,
        });
        self.run_all_fire_forget(&self.hooks.session_end, input)
            .await;
    }

    /// Fire when a desktop notification is sent (non-blocking).
    pub async fn notification(&self, message: &str, level: &str) {
        let input = json!({
            "event_type": "Notification",
            "message":    message,
            "level":      level,
        });
        self.run_all_fire_forget(&self.hooks.notification, input)
            .await;
    }
}

// -- Internal dispatch

impl HookEngine {
    /// Run all entries that match `tool_name`. First exit-2 blocks; returns outcome.
    async fn run_entries_blocking(
        &self,
        entries: &[crate::settings::HookEntry],
        tool_name: &str,
        input: Value,
    ) -> HookOutcome {
        for entry in entries {
            if !matcher_matches(&entry.matcher, tool_name) {
                continue;
            }
            for hook in &entry.hooks {
                match run_hook_command(hook, &input, &self.cwd).await {
                    HookResult::Block(reason) => return HookOutcome::Block { reason },
                    HookResult::Allow | HookResult::Continue => {}
                }
            }
        }
        HookOutcome::Allow
    }

    /// Same as above but for non-tool events (no matcher).
    async fn run_all_blocking(
        &self,
        entries: &[crate::settings::HookEntry],
        input: Value,
    ) -> HookOutcome {
        for entry in entries {
            for hook in &entry.hooks {
                match run_hook_command(hook, &input, &self.cwd).await {
                    HookResult::Block(reason) => return HookOutcome::Block { reason },
                    HookResult::Allow | HookResult::Continue => {}
                }
            }
        }
        HookOutcome::Allow
    }

    /// Run PostToolUse hooks; collect additionalContext from stdout JSON.
    async fn run_entries_context(
        &self,
        entries: &[crate::settings::HookEntry],
        tool_name: &str,
        input: Value,
    ) -> Option<String> {
        let mut context: Option<String> = None;
        for entry in entries {
            if !matcher_matches(&entry.matcher, tool_name) {
                continue;
            }
            for hook in &entry.hooks {
                if let Some(extra) = run_hook_command_with_context(hook, &input, &self.cwd).await {
                    context = Some(match context {
                        Some(existing) => format!("{existing}\n{extra}"),
                        None => extra,
                    });
                }
            }
        }
        context
    }

    async fn run_entries_fire_forget(
        &self,
        entries: &[crate::settings::HookEntry],
        tool_name: &str,
        input: Value,
    ) {
        for entry in entries {
            if !matcher_matches(&entry.matcher, tool_name) {
                continue;
            }
            for hook in &entry.hooks {
                let _ = run_hook_command(hook, &input, &self.cwd).await;
            }
        }
    }

    async fn run_all_fire_forget(
        &self,
        entries: &[crate::settings::HookEntry],
        input: Value,
    ) {
        for entry in entries {
            for hook in &entry.hooks {
                let _ = run_hook_command(hook, &input, &self.cwd).await;
            }
        }
    }
}

// -- Matcher

/// Returns true if `tool_name` matches `matcher`.
/// None / "" / "*" → match all. Otherwise treated as a regex.
fn matcher_matches(matcher: &Option<String>, tool_name: &str) -> bool {
    match matcher.as_deref() {
        None | Some("") | Some("*") => true,
        Some(pat) => {
            // Simple regex: split on "|" for alternation, support ".*" wildcard
            regex_match(pat, tool_name)
        }
    }
}

fn regex_match(pattern: &str, text: &str) -> bool {
    // Use the regex crate for full pattern support (anchoring, character classes,
    // groups, etc.).  Compile on each call — hooks are checked on tool events, not
    // in hot loops, so per-call compilation is acceptable.
    // Fall back to a plain case-insensitive substring match if the pattern is not
    // valid regex so that simple literal matchers still work without escaping.
    match regex::Regex::new(pattern) {
        Ok(re) => re.is_match(text),
        Err(_) => text.to_lowercase().contains(&pattern.to_lowercase()),
    }
}

// -- Command runner

#[derive(Debug)]
enum HookResult {
    Allow,         // exit 0
    Continue,      // exit 1 — log, action proceeds
    Block(String), // exit 2 — stderr shown to agent
}

async fn run_hook_command(
    hook: &crate::settings::HookDef,
    input: &Value,
    cwd: &PathBuf,
) -> HookResult {
    let crate::settings::HookDef::Command { command, timeout } = hook;
    let timeout_ms = *timeout;
    let input_str = serde_json::to_string(input).unwrap_or_default();

    let result = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        spawn_command(command, &input_str, cwd),
    )
    .await;

    match result {
        Err(_) => {
            tracing::warn!("Hook timed out after {timeout_ms}ms: {command}");
            HookResult::Continue
        }
        Ok(Err(e)) => {
            tracing::warn!("Hook failed to spawn: {command}: {e}");
            HookResult::Continue
        }
        Ok(Ok((exit_code, stdout, stderr))) => match exit_code {
            0 => {
                if !stdout.is_empty() {
                    tracing::debug!("Hook stdout: {}", stdout.trim());
                }
                HookResult::Allow
            }
            1 => {
                tracing::warn!("Hook non-blocking error: {command}: {}", stderr.trim());
                HookResult::Continue
            }
            2 => {
                let reason = if stderr.trim().is_empty() {
                    format!("Hook blocked: {command}")
                } else {
                    stderr.trim().to_string()
                };
                tracing::info!("Hook blocked action: {reason}");
                HookResult::Block(reason)
            }
            other => {
                tracing::warn!("Hook unexpected exit {other}: {command}");
                HookResult::Continue
            }
        },
    }
}

/// Run a PostToolUse hook and return additionalContext if stdout contains it.
async fn run_hook_command_with_context(
    hook: &crate::settings::HookDef,
    input: &Value,
    cwd: &PathBuf,
) -> Option<String> {
    let crate::settings::HookDef::Command { command, timeout } = hook;
    let input_str = serde_json::to_string(input).unwrap_or_default();

    let result = tokio::time::timeout(
        Duration::from_millis(*timeout),
        spawn_command(command, &input_str, cwd),
    )
    .await;

    match result {
        Ok(Ok((0, stdout, _))) if !stdout.trim().is_empty() => {
            // Parse stdout for additionalContext
            if let Ok(v) = serde_json::from_str::<Value>(&stdout) {
                let ctx = v
                    .get("additionalContext")
                    .or_else(|| v.pointer("/hookSpecificOutput/additionalContext"))
                    .and_then(|c| c.as_str())
                    .map(String::from);
                return ctx;
            }
            None
        }
        Ok(Ok((1, _, stderr))) => {
            tracing::warn!("PostToolUse hook non-blocking error: {}", stderr.trim());
            None
        }
        Ok(Ok((2, _, stderr))) => {
            tracing::warn!(
                "PostToolUse hook exit 2 (ignored for PostToolUse): {}",
                stderr.trim()
            );
            None
        }
        _ => None,
    }
}

async fn spawn_command(
    command: &str,
    stdin_data: &str,
    cwd: &PathBuf,
) -> crate::Result<(i32, String, String)> {
    use tokio::io::AsyncWriteExt;

    let mut child = {
        let mut cmd = crate::shell::shell_command(command);
        crate::agent_env::apply_agent_env(&mut cmd);
        cmd.current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_data.as_bytes()).await;
    }

    let output = child.wait_with_output().await?;
    let exit_code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok((exit_code, stdout, stderr))
}

// endregion: --- Tests

// -- Display helpers

impl std::fmt::Display for crate::settings::HookDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Command { command, timeout } = self;
        if *timeout == 60_000 {
            write!(f, "[command] {command}")
        } else {
            write!(f, "[command] {command}  (timeout: {timeout}ms)")
        }
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use crate::settings::{HookDef, HookEntry, HooksConfig};
    use std::path::PathBuf;

    // -- HookOutcome

    #[test]
    fn hook_outcome_allow_is_not_block() {
        let outcome = HookOutcome::Allow;
        assert!(!outcome.is_block());
        assert!(outcome.reason().is_none());
    }

    #[test]
    fn hook_outcome_block_with_reason() {
        let outcome = HookOutcome::Block {
            reason: "denied".into(),
        };
        assert!(outcome.is_block());
        assert_eq!(outcome.reason(), Some("denied"));
    }

    // -- HookEngine::is_empty

    #[test]
    fn engine_empty_when_no_hooks() {
        let engine = HookEngine::new(HooksConfig::default(), PathBuf::from("."));
        assert!(engine.is_empty());
    }

    #[test]
    fn engine_not_empty_with_hooks() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: "echo ok".into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        assert!(!engine.is_empty());
    }

    // -- matcher_matches

    #[test]
    fn matcher_none_matches_all() {
        assert!(matcher_matches(&None, "bash"));
        assert!(matcher_matches(&None, "anything"));
    }

    #[test]
    fn matcher_empty_string_matches_all() {
        assert!(matcher_matches(&Some("".into()), "bash"));
    }

    #[test]
    fn matcher_star_matches_all() {
        assert!(matcher_matches(&Some("*".into()), "bash"));
    }

    #[test]
    fn matcher_exact_regex() {
        assert!(matcher_matches(&Some("bash".into()), "bash"));
        assert!(!matcher_matches(&Some("^bash$".into()), "bash_extended"));
    }

    #[test]
    fn matcher_alternation() {
        assert!(matcher_matches(&Some("bash|edit_file".into()), "bash"));
        assert!(matcher_matches(&Some("bash|edit_file".into()), "edit_file"));
        assert!(!matcher_matches(
            &Some("bash|edit_file".into()),
            "read_file"
        ));
    }

    #[test]
    fn matcher_invalid_regex_falls_back_to_substring() {
        // "[" is invalid regex — should fall back to case-insensitive substring
        assert!(matcher_matches(&Some("[".into()), "a[b"));
    }

    // -- HooksConfig::merge

    #[test]
    fn hooks_config_merge() {
        let entry_a = HookEntry {
            matcher: Some("bash".into()),
            hooks: vec![HookDef::Command {
                command: "echo a".into(),
                timeout: 5000,
            }],
        };
        let entry_b = HookEntry {
            matcher: Some("edit_file".into()),
            hooks: vec![HookDef::Command {
                command: "echo b".into(),
                timeout: 5000,
            }],
        };

        let mut a = HooksConfig::default();
        a.pre_tool_use.push(entry_a);

        let mut b = HooksConfig::default();
        b.pre_tool_use.push(entry_b);

        let merged = a.merge(b);
        assert_eq!(merged.pre_tool_use.len(), 2);
    }

    #[test]
    fn hooks_config_is_empty_default() {
        assert!(HooksConfig::default().is_empty());
    }

    // -- Integration: hook runs a real command

    #[tokio::test]
    async fn pre_tool_use_allows_on_exit_0() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: "exit 0".into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        let outcome = engine.pre_tool_use("bash", &json!({})).await;
        assert!(!outcome.is_block());
    }

    #[tokio::test]
    async fn pre_tool_use_blocks_on_exit_2() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: "echo 'forbidden' >&2; exit 2".into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        let outcome = engine.pre_tool_use("bash", &json!({})).await;
        assert!(outcome.is_block());
        assert_eq!(outcome.reason(), Some("forbidden"));
    }

    #[tokio::test]
    async fn pre_tool_use_continues_on_exit_1() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: "exit 1".into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        let outcome = engine.pre_tool_use("bash", &json!({})).await;
        assert!(!outcome.is_block());
    }

    #[tokio::test]
    async fn post_tool_use_extracts_additional_context() {
        let mut config = HooksConfig::default();
        config.post_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: r#"echo '{"additionalContext":"extra info"}'"#.into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        let ctx = engine
            .post_tool_use("bash", &json!({}), "output", None, None)
            .await;
        assert_eq!(ctx.as_deref(), Some("extra info"));
    }

    #[tokio::test]
    async fn matcher_filters_tool_name() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: Some("^edit_file$".into()),
            hooks: vec![HookDef::Command {
                command: "exit 2".into(),
                timeout: 5000,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        // bash should not match ^edit_file$ → allow
        let outcome = engine.pre_tool_use("bash", &json!({})).await;
        assert!(!outcome.is_block());
    }

    #[tokio::test]
    async fn hook_timeout() {
        let mut config = HooksConfig::default();
        config.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![HookDef::Command {
                command: "sleep 10".into(),
                timeout: 100,
            }],
        });
        let engine = HookEngine::new(config, PathBuf::from("."));
        let outcome = engine.pre_tool_use("bash", &json!({})).await;
        // Timeout → treated as Continue (Allow)
        assert!(!outcome.is_block());
    }
}

/// Parsed configuration for a single `run_subagent` tool call.
///
/// `SubagentConfig` is the single source of truth for all fields an agent can
/// pass to `run_subagent`.  It lives in `cade-agent` so both the CLI and the
/// server can share argument parsing, validation, system-prompt construction,
/// seed-memory assembly, and model resolution without duplicating logic.
///
/// **Design constraints:**
/// - No `cade_ai` dependency — `resolve_model` returns an `Option<String>` so
///   callers supply their own `fast_model_for_main_model` fallback.
/// - No I/O — all methods are pure; callers are responsible for reading memory
///   blocks from the DB or HTTP transport before calling `build_seed_memory`.
use serde_json::Value;

use crate::agent::client::MemoryBlock;
use crate::subagents::SubagentDef;

// ── Seed memory cap ──────────────────────────────────────────────────────────

/// Maximum characters kept per block when seeding subagent context.
const SEED_BLOCK_MAX_CHARS: usize = 1500;

/// Labels that are internal bookkeeping and must not be forwarded to subagents.
const SKIP_LABELS: &[&str] = &["active_goal", "session_summary"];

// ── SubagentConfig ───────────────────────────────────────────────────────────

/// All fields that can be provided via `run_subagent` tool arguments.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// The task description forwarded to the subagent.  Required — callers
    /// should validate with [`SubagentConfig::validate`] before using.
    pub prompt: String,

    /// Subagent mode / name.  Either a custom subagent name (looked up in
    /// `~/.cade/subagents/` / `.cade/subagents/`) or a built-in key like
    /// `"build"` / `"plan"`.  Defaults to `"build"`.
    pub mode: String,

    /// When true the subagent runs in a background task; the tool call
    /// returns immediately with a task ID.
    pub background: bool,

    /// Optional model override supplied by the caller.  When `None` the
    /// caller should fall back to a fast model or the parent agent's model.
    pub model_override: Option<String>,

    /// Caller-supplied system prompt.  Highest priority in resolution chain.
    pub custom_system_prompt: Option<String>,

    /// Human-readable label for the ephemeral agent.
    pub description: Option<String>,

    /// Shell command to run after the subagent finishes as proof-of-work.
    pub test_command: Option<String>,

    /// Deploy an existing stateful agent (by ID) instead of creating an
    /// ephemeral one.
    pub agent_id: Option<String>,

    /// When true the parent agent is prompted to approve / reject the result
    /// before it is returned.
    pub human_review: bool,

    /// Suppress streaming output from this subagent run.
    pub silent_stream: bool,

    /// Depth counter injected by the dispatcher when a subagent itself calls
    /// `run_subagent`.  Used to enforce the recursion depth cap.
    pub depth: usize,

    /// Maximum combined tokens (prompt + completion) this subagent is allowed to consume.
    pub max_tokens_budget: Option<u64>,
}

impl SubagentConfig {
    // ── Parsing ──────────────────────────────────────────────────────────────

    /// Parse a `SubagentConfig` from the raw JSON arguments of a `run_subagent`
    /// tool call.
    ///
    /// Does **not** validate that `prompt` is non-empty — call
    /// [`Self::validate`] after construction when an error `ToolResult` is
    /// wanted.
    pub fn from_args(args: &Value) -> Self {
        let prompt = args["prompt"].as_str().unwrap_or("").trim().to_string();

        let mode = args["mode"].as_str().unwrap_or("build").trim().to_string();

        let background = args["background"].as_bool().unwrap_or(false);

        let model_override = args["model"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let custom_system_prompt = args["system_prompt"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let description = args["description"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let test_command = args["test_command"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let agent_id = args["agent_id"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let human_review = args["human_review"].as_bool().unwrap_or(false);
        let silent_stream = args["silent_stream"].as_bool().unwrap_or(false);

        let depth = args["_subagent_depth"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(0);

        let max_tokens_budget = args["max_tokens_budget"].as_u64();

        Self {
            prompt,
            mode,
            background,
            model_override,
            custom_system_prompt,
            description,
            test_command,
            agent_id,
            human_review,
            silent_stream,
            depth,
            max_tokens_budget,
        }
    }

    // ── Validation ───────────────────────────────────────────────────────────

    /// Returns `Err(reason)` if the config is invalid (e.g., prompt empty).
    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.trim().is_empty() {
            return Err("error: 'prompt' is required".to_string());
        }
        Ok(())
    }

    // ── Tool resolution ──────────────────────────────────────────────────────

    pub fn resolve_allowed_paths(&self, def: Option<&SubagentDef>) -> Option<Vec<String>> {
        match def
            .map(|d| &d.tools)
            .unwrap_or(&crate::subagents::SubagentTools::All)
        {
            crate::subagents::SubagentTools::Restricted { allowed_paths, .. } => {
                Some(allowed_paths.clone())
            }
            _ => None,
        }
    }

    /// Build the final system prompt from the resolution chain:
    ///
    /// 1. `custom_system_prompt` (highest priority — caller-supplied)
    /// 2. `def.system_prompt` (named subagent definition from `.md` file)
    /// 3. Built-in default (worker prose)
    ///
    /// For `mode == "plan"` a strict read-only directive is always appended
    /// regardless of which source supplied the base.
    ///
    /// The `prompt` is **not** appended here; callers add it as a separate
    /// `user` message so the LLM context is cleanly separated.
    pub fn resolve_system_prompt(&self, def: Option<&SubagentDef>) -> String {
        const DEFAULT_WORKER: &str = "\
You are a CADE subagent — an autonomous coding assistant spawned to complete a \
focused task. You have access to the full CADE tool suite (file operations, shell, \
search, memory). Use tools aggressively to gather information before answering. \
Return a concise final summary describing what you did, what you found, and any \
follow-ups for the parent agent. Do NOT ask the parent clarifying questions — make \
reasonable assumptions and proceed.";

        const HEADLESS_OVERRIDE: &str = "\n\nCRITICAL SYSTEM OVERRIDE: You are \
running in a headless autonomous loop. You MUST call tools to accomplish the task. \
Do NOT ask for permission or emit conversational filler without calling a tool. If \
you output plain text without a tool call, your execution terminates immediately. \
When the task is complete, summarize your findings and stop.";

        const PLAN_DIRECTIVE: &str = "\n\nIMPORTANT: You are in PLAN mode. Analyze \
and report only. Do NOT modify files, do NOT run mutating commands, do NOT use \
write_file / edit_file / apply_patch.";

        let base = self
            .custom_system_prompt
            .clone()
            .or_else(|| def.map(|d| d.system_prompt.clone()))
            .unwrap_or_else(|| DEFAULT_WORKER.to_string());

        let mut prompt = format!("{base}{HEADLESS_OVERRIDE}");

        if self.mode == "plan" {
            prompt.push_str(PLAN_DIRECTIVE);
        }

        prompt
    }

    // ── Seed memory ──────────────────────────────────────────────────────────

    /// Filter and cap a slice of parent memory blocks into seed blocks for the
    /// subagent's initial context.
    ///
    /// Rules (same in both CLI and server paths, now in one place):
    /// - Skip `__`-prefixed labels (internal bookkeeping).
    /// - Skip orchestration labels (`active_goal`, `session_summary`).
    /// - Only include `pinned` or `short`-tier blocks (or blocks with no tier).
    /// - Skip empty values.
    /// - Cap each value at [`SEED_BLOCK_MAX_CHARS`] characters.
    /// - Clear the tier field so the server assigns its own default.
    pub fn build_seed_memory(parent_blocks: Vec<MemoryBlock>) -> Vec<MemoryBlock> {
        parent_blocks
            .into_iter()
            .filter(|b| {
                if b.label.starts_with("__") {
                    return false;
                }
                if SKIP_LABELS.contains(&b.label.as_str()) {
                    return false;
                }
                let tier_ok = b
                    .tier
                    .as_deref()
                    .is_none_or(|t| t == "pinned" || t == "short");
                if !tier_ok {
                    return false;
                }
                !b.value.trim().is_empty()
            })
            .map(|b| {
                let value = cap_chars(&b.value, SEED_BLOCK_MAX_CHARS);
                MemoryBlock {
                    label: b.label,
                    value,
                    description: b.description,
                    tier: None, // let the server assign its default
                }
            })
            .collect()
    }

    /// Format seeded blocks as a system-prompt section (used by the server path
    /// which injects memory into the system prompt rather than DB rows).
    pub fn format_seed_section(seed_blocks: &[MemoryBlock]) -> String {
        if seed_blocks.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n\n# Inherited memory (from parent agent)\n");
        for b in seed_blocks {
            if !b.value.trim().is_empty() {
                out.push_str(&format!("\n## {}\n{}\n", b.label, b.value));
            }
        }
        out
    }

    // ── Model resolution ─────────────────────────────────────────────────────

    /// Return the model to use for this subagent, applying the preference chain:
    ///
    /// 1. Caller's explicit `model` argument.
    /// 2. Named definition's `model` field (if any).
    /// 3. `None` — caller must supply its own fallback (e.g.
    ///    `cade_ai::catalogue::fast_model_for_main_model`).
    ///
    /// Not resolving to a final string here keeps `cade-agent` free of the
    /// `cade_ai` dependency.
    pub fn resolve_model<'a>(&'a self, def: Option<&'a SubagentDef>) -> Option<&'a str> {
        self.model_override
            .as_deref()
            .or_else(|| def.and_then(|d| d.model.as_deref()))
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Append the test-command proof-of-work instruction to the prompt.
    /// Callers do this before forwarding the prompt to the subagent.
    pub fn prompt_with_test_command(&self) -> String {
        match &self.test_command {
            Some(cmd) => format!(
                "{}\n\nCRITICAL PROOF OF WORK REQUIRED: You MUST run the following \
                 test command to verify your fix: `{cmd}`. Do not return until this \
                 command passes. The main agent will execute this command on the host \
                 system to verify your work. If it fails, your answer will be rejected.",
                self.prompt
            ),
            None => self.prompt.clone(),
        }
    }

    /// Name to use when creating an ephemeral agent for this subagent run.
    pub fn ephemeral_agent_name(&self, task_id: &str) -> String {
        format!("subagent-{}-{}", self.mode, task_id)
    }

    /// Human-readable description for the ephemeral agent.
    pub fn ephemeral_description(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| format!("Ephemeral subagent: {}", self.mode))
    }
}

// ── Utilities ────────────────────────────────────────────────────────────────

/// Truncate `s` to at most `max` Unicode scalar values, appending `…` when cut.
fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}…", &s[..end])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- from_args

    #[test]
    fn config_from_args_defaults() {
        let args = json!({ "prompt": "do the thing" });
        let cfg = SubagentConfig::from_args(&args);
        assert_eq!(cfg.prompt, "do the thing");
        assert_eq!(cfg.mode, "build");
        assert!(!cfg.background);
        assert!(cfg.model_override.is_none());
        assert!(cfg.custom_system_prompt.is_none());
        assert!(cfg.description.is_none());
        assert!(cfg.test_command.is_none());
        assert!(cfg.agent_id.is_none());
        assert!(!cfg.human_review);
        assert!(!cfg.silent_stream);
        assert_eq!(cfg.depth, 0);
    }

    #[test]
    fn config_from_args_full() {
        let args = json!({
            "prompt":        "search for bugs",
            "mode":          "plan",
            "background":    true,
            "model":         "gemini-2.5-pro",
            "system_prompt": "Be brief.",
            "description":   "bug hunter",
            "test_command":  "cargo test",
            "agent_id":      "agent-abc",
            "human_review":  true,
            "silent_stream": true,
            "_subagent_depth": 2,
        });
        let cfg = SubagentConfig::from_args(&args);
        assert_eq!(cfg.prompt, "search for bugs");
        assert_eq!(cfg.mode, "plan");
        assert!(cfg.background);
        assert_eq!(cfg.model_override.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(cfg.custom_system_prompt.as_deref(), Some("Be brief."));
        assert_eq!(cfg.description.as_deref(), Some("bug hunter"));
        assert_eq!(cfg.test_command.as_deref(), Some("cargo test"));
        assert_eq!(cfg.agent_id.as_deref(), Some("agent-abc"));
        assert!(cfg.human_review);
        assert!(cfg.silent_stream);
        assert_eq!(cfg.depth, 2);
    }

    #[test]
    fn config_empty_strings_treated_as_none() {
        let args = json!({ "prompt": "x", "model": "   ", "system_prompt": "" });
        let cfg = SubagentConfig::from_args(&args);
        assert!(
            cfg.model_override.is_none(),
            "whitespace-only model must be None"
        );
        assert!(
            cfg.custom_system_prompt.is_none(),
            "empty system_prompt must be None"
        );
    }

    // -- validate

    #[test]
    fn config_validate_rejects_empty_prompt() {
        let cfg = SubagentConfig::from_args(&json!({}));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn config_validate_accepts_non_empty_prompt() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "do something" }));
        assert!(cfg.validate().is_ok());
    }

    // -- resolve_system_prompt

    #[test]
    fn resolve_system_prompt_custom_takes_priority() {
        let cfg = SubagentConfig::from_args(&json!({
            "prompt": "x",
            "system_prompt": "custom instructions",
        }));
        let result = cfg.resolve_system_prompt(None);
        assert!(
            result.contains("custom instructions"),
            "custom prompt must appear in result"
        );
    }

    #[test]
    fn resolve_system_prompt_def_used_when_no_custom() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x" }));
        let def = SubagentDef {
            name: "my-worker".to_string(),
            description: "test".to_string(),
            model: None,
            tools: crate::subagents::SubagentTools::All,
            system_prompt: "def instructions".to_string(),
            skills: vec![],
            scope: crate::subagents::SubagentScope::Builtin,
            path: None,
        };
        let result = cfg.resolve_system_prompt(Some(&def));
        assert!(result.contains("def instructions"));
    }

    #[test]
    fn resolve_system_prompt_plan_mode_appends_directive() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x", "mode": "plan" }));
        let result = cfg.resolve_system_prompt(None);
        assert!(
            result.contains("PLAN mode"),
            "plan mode directive must be present"
        );
    }

    #[test]
    fn resolve_system_prompt_build_mode_no_plan_directive() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x", "mode": "build" }));
        let result = cfg.resolve_system_prompt(None);
        assert!(!result.contains("PLAN mode"));
    }

    #[test]
    fn resolve_system_prompt_contains_headless_override() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x" }));
        let result = cfg.resolve_system_prompt(None);
        assert!(result.contains("headless autonomous loop"));
    }

    // -- build_seed_memory

    fn mb(label: &str, value: &str, tier: Option<&str>) -> MemoryBlock {
        MemoryBlock {
            label: label.to_string(),
            value: value.to_string(),
            description: None,
            tier: tier.map(|s| s.to_string()),
        }
    }

    #[test]
    fn build_seed_memory_keeps_pinned_and_short() {
        let blocks = vec![
            mb("project", "proj info", Some("pinned")),
            mb("human", "user prefs", Some("short")),
            mb("old_notes", "stale", Some("long")),
        ];
        let seed = SubagentConfig::build_seed_memory(blocks);
        let labels: Vec<&str> = seed.iter().map(|b| b.label.as_str()).collect();
        assert!(labels.contains(&"project"));
        assert!(labels.contains(&"human"));
        assert!(
            !labels.contains(&"old_notes"),
            "long-tier must be filtered out"
        );
    }

    #[test]
    fn build_seed_memory_skips_internal_labels() {
        let blocks = vec![
            mb("active_goal", "some goal", Some("pinned")),
            mb("session_summary", "some summary", Some("short")),
            mb("__internal", "hidden", None),
            mb("project", "keeps", Some("pinned")),
        ];
        let seed = SubagentConfig::build_seed_memory(blocks);
        let labels: Vec<&str> = seed.iter().map(|b| b.label.as_str()).collect();
        assert!(!labels.contains(&"active_goal"));
        assert!(!labels.contains(&"session_summary"));
        assert!(!labels.contains(&"__internal"));
        assert!(labels.contains(&"project"));
    }

    #[test]
    fn build_seed_memory_caps_long_values() {
        let long_value = "x".repeat(2000);
        let blocks = vec![mb("big", &long_value, None)];
        let seed = SubagentConfig::build_seed_memory(blocks);
        assert_eq!(seed.len(), 1);
        // Value must be capped — char count ≤ SEED_BLOCK_MAX_CHARS + ellipsis
        assert!(seed[0].value.chars().count() <= SEED_BLOCK_MAX_CHARS + 1);
        assert!(seed[0].value.ends_with('…'));
    }

    #[test]
    fn build_seed_memory_skips_empty_values() {
        let blocks = vec![
            mb("noisy", "", Some("pinned")),
            mb("noisy2", "   ", Some("short")),
            mb("ok", "content", None),
        ];
        let seed = SubagentConfig::build_seed_memory(blocks);
        assert_eq!(seed.len(), 1);
        assert_eq!(seed[0].label, "ok");
    }

    #[test]
    fn build_seed_memory_clears_tier() {
        let blocks = vec![mb("project", "info", Some("pinned"))];
        let seed = SubagentConfig::build_seed_memory(blocks);
        assert!(
            seed[0].tier.is_none(),
            "tier must be cleared in seed blocks"
        );
    }

    // -- resolve_model

    #[test]
    fn resolve_model_explicit_arg_wins() {
        let cfg = SubagentConfig::from_args(&json!({
            "prompt": "x",
            "model": "gpt-4o",
        }));
        assert_eq!(cfg.resolve_model(None), Some("gpt-4o"));
    }

    #[test]
    fn resolve_model_falls_back_to_def() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x" }));
        let def = SubagentDef {
            name: "w".to_string(),
            description: String::new(),
            model: Some("claude-haiku-3-5".to_string()),
            tools: crate::subagents::SubagentTools::All,
            system_prompt: String::new(),
            skills: vec![],
            scope: crate::subagents::SubagentScope::Builtin,
            path: None,
        };
        assert_eq!(cfg.resolve_model(Some(&def)), Some("claude-haiku-3-5"));
    }

    #[test]
    fn resolve_model_returns_none_when_no_hint() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "x" }));
        assert!(cfg.resolve_model(None).is_none());
    }

    // -- prompt_with_test_command

    #[test]
    fn prompt_with_test_command_appends_instruction() {
        let cfg = SubagentConfig::from_args(&json!({
            "prompt": "fix the bug",
            "test_command": "cargo test",
        }));
        let full = cfg.prompt_with_test_command();
        assert!(full.starts_with("fix the bug"));
        assert!(full.contains("CRITICAL PROOF OF WORK REQUIRED"));
        assert!(full.contains("cargo test"));
    }

    #[test]
    fn prompt_with_test_command_unchanged_without_cmd() {
        let cfg = SubagentConfig::from_args(&json!({ "prompt": "just do it" }));
        assert_eq!(cfg.prompt_with_test_command(), "just do it");
    }

    // -- cap_chars utility

    #[test]
    fn cap_chars_short_string_unchanged() {
        assert_eq!(cap_chars("hello", 10), "hello");
    }

    #[test]
    fn cap_chars_long_string_truncated() {
        let result = cap_chars("abcdefghij", 5);
        assert_eq!(result, "abcde…");
    }

    #[test]
    fn cap_chars_multibyte_safe() {
        // Each '©' is 2 bytes but 1 char
        let s = "©".repeat(10);
        let result = cap_chars(&s, 5);
        assert!(result.ends_with('…'));
        // Must not panic or produce invalid UTF-8
        assert_eq!(result.chars().filter(|&c| c == '©').count(), 5);
    }
}

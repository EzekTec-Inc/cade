use std::sync::Arc;
use parking_lot::Mutex;
use crate::permissions::rules::*;
use crate::permissions::checks::*;

// -- PermissionManager

#[derive(Clone, Default)]
pub struct PermissionManager {
    mode: Arc<Mutex<PermissionMode>>,
    allow_rules: Arc<Mutex<Vec<PermissionRule>>>,
    deny_rules: Arc<Mutex<Vec<PermissionRule>>>,
    /// SEC-B1: When true, bash tools are never auto-approved.
    strict_bash: bool,
}

impl PermissionManager {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode: Arc::new(Mutex::new(mode)),
            allow_rules: Arc::new(Mutex::new(Vec::new())),
            deny_rules: Arc::new(Mutex::new(Vec::new())),
            strict_bash: false,
        }
    }

    /// Construct with the strict_bash flag pre-set.
    pub fn new_with_strict_bash(mode: PermissionMode, strict_bash: bool) -> Self {
        Self {
            mode: Arc::new(Mutex::new(mode)),
            allow_rules: Arc::new(Mutex::new(Vec::new())),
            deny_rules: Arc::new(Mutex::new(Vec::new())),
            strict_bash,
        }
    }

    pub fn mode(&self) -> PermissionMode {
        *self.mode.lock()
    }
    pub fn set_mode(&self, mode: PermissionMode) {
        *self.mode.lock() = mode;
    }

    pub fn add_allow_rule(&self, rule: PermissionRule) {
        let mut rules = self.allow_rules.lock();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }

    pub fn add_deny_rule(&self, rule: PermissionRule) {
        let mut rules = self.deny_rules.lock();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }

    /// Add a session-scope allow rule by raw string (e.g. from `A` keypress in prompt).
    /// Parses the string; silently ignores invalid rules.
    pub fn add_session_allow(&self, raw: &str) {
        if let Some(rule) = PermissionRule::parse(raw) {
            self.add_allow_rule(rule);
        }
    }

    /// Clear all rules, then load new ones from the given settings.
    /// Note: This resets any session-level allow rules.
    pub fn reload_from_settings(&self, settings: &crate::settings::manager::PermissionSettings) {
        self.allow_rules.lock().clear();
        self.deny_rules.lock().clear();
        for raw in &settings.allow {
            if let Some(rule) = PermissionRule::parse(raw) {
                self.add_allow_rule(rule);
            }
        }
        for raw in &settings.deny {
            if let Some(rule) = PermissionRule::parse(raw) {
                self.add_deny_rule(rule);
            }
        }
    }

    pub fn allow_rules(&self) -> Vec<PermissionRule> {
        self.allow_rules.lock().clone()
    }
    pub fn deny_rules(&self) -> Vec<PermissionRule> {
        self.deny_rules.lock().clone()
    }

    /// Unified permission resolution.
    ///
    /// Resolution order (highest priority first):
    ///   1. Protected path write        → Deny (always, any mode)
    ///   2. Explicit deny_rules match   → Deny
    ///   3. Explicit allow_rules match  → Allow
    ///   4. SEC-B1: strict_bash         → Ask
    ///   5. SEC-B3: config/skill edits  → Ask
    ///   6. Mode-based:
    ///      - Bypass         → Allow (with audit log)
    ///      - Plan           → Deny for writes, Allow for reads
    ///      - AcceptEdits    → Allow for create/edit, Ask for delete
    ///      - Default        → Ask for writes, Allow for reads
    ///   7. Fallback                    → Allow (read-only tools)
    pub fn resolve(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        is_mcp_write: bool,
    ) -> Verdict {
        let arg = tool_first_arg(tool_name, args);
        let arg_ref: Option<&str> = arg.as_deref();

        let base_name = if let Some(pos) = tool_name.rfind("__") {
            &tool_name[pos + 2..]
        } else {
            tool_name
        };

        let is_bash = matches!(base_name, "bash");

        let is_write = is_write_schema(base_name) || is_mcp_write;

        let bash_is_write = if is_bash {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            bash_command_is_write(cmd)
        } else {
            false
        };

        // 1. Protected path — hard-block writes always
        if let Some(arg_str) = arg_ref
            && path_is_protected(arg_str)
            && (is_write || bash_is_write)
        {
            return Verdict::Deny(
                "security: protected path access denied (.git, .env, .ssh)".to_string(),
            );
        }

        // 2. Explicit deny rules — hard-block
        if self
            .deny_rules
            .lock()
            .iter()
            .any(|r| r.matches(tool_name, arg_ref))
        {
            let rule = self
                .deny_rules
                .lock()
                .iter()
                .find(|r| r.matches(tool_name, arg_ref))
                .cloned();
            return Verdict::Deny(format!(
                "blocked by deny rule: {}",
                rule.map(|r| r.to_string()).unwrap_or_default()
            ));
        }

        // 2.5. Plan mode strict block — overrides any allow rules for mutations
        if self.mode() == PermissionMode::Plan {
            if is_write {
                return Verdict::Deny(format!(
                    "plan mode: '{tool_name}' is a write/mutating tool"
                ));
            }
            if is_bash && bash_is_write {
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                return Verdict::Deny(format!(
                    "plan mode: '{}' would modify system state",
                    cmd.chars().take(60).collect::<String>()
                ));
            }
        }

        // 3. Explicit allow rules
        if self
            .allow_rules
            .lock()
            .iter()
            .any(|r| r.matches(tool_name, arg_ref))
        {
            // SEC-B1: strict_bash overrides allow rules for bash tools
            if self.strict_bash && is_bash {
                return Verdict::Ask("strict_bash: bash tools always require approval".to_string());
            }
            return Verdict::Allow;
        }

        // 4. SEC-B1: strict_bash — never auto-approve bash tools
        if self.strict_bash && is_bash {
            return Verdict::Ask("strict_bash: bash tools always require approval".to_string());
        }

        // 5. SEC-B3: Prevent auto-approval of config/skill edits (RCE mitigation)
        if matches!(
            base_name,
            "write_file" | "edit_file" | "apply_patch" | "write" | "edit" | "patch" | "edit_block"
        ) && let Some(path) = arg_ref
            && (path.contains(".cade/settings.json")
                || path.contains("settings.local.json")
                || path.contains(".cade/skills/"))
        {
            return Verdict::Ask(
                "security: config/skill edits require explicit approval".to_string(),
            );
        }

        // 6. Mode-based resolution
        match self.mode() {
            PermissionMode::BypassPermissions => {
                tracing::warn!(
                    "bypassPermissions: auto-approving tool '{}' arg={:?}",
                    tool_name,
                    arg.as_deref().unwrap_or("<none>")
                );
                Verdict::Allow
            }

            PermissionMode::Plan => {
                Verdict::Allow
            }

            PermissionMode::AcceptEdits => {
                // Delete actions always require user approval
                if is_delete_action(tool_name, base_name, args, is_mcp_write) {
                    return Verdict::Ask(
                        "delete action requires approval in acceptEdits mode".to_string(),
                    );
                }
                // Non-delete writes are auto-approved
                if is_write || bash_is_write {
                    return Verdict::Allow;
                }
                // Read-only tools are always allowed
                Verdict::Allow
            }

            PermissionMode::Default => {
                if is_write || bash_is_write {
                    let reason = if is_bash {
                        let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                        format!(
                            "default mode: '{}' requires approval",
                            cmd.chars().take(60).collect::<String>()
                        )
                    } else {
                        format!("default mode: '{tool_name}' requires approval")
                    };
                    return Verdict::Ask(reason);
                }
                Verdict::Allow
            }
        }
    }
}

// endregion: --- Tests

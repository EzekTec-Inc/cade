// region:    --- Modules

use std::sync::Arc;
use parking_lot::Mutex;

// endregion: --- Modules

// -- PermissionRule

/// A single allow or deny rule matching a tool call.
///
/// Syntax mirrors CADE Code:
///   `Bash`              — all uses of the bash tool
///   `Bash(cargo test)`  — bash where command == "cargo test"
///   `Read(src/**)`      — read_file where path starts with "src/" (glob **)
///   `Bash(rm -rf:*)`    — bash where command starts with "rm -rf" (:* suffix wildcard)
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRule {
    /// Tool name, lower-cased for comparison (e.g. "bash", "edit_file")
    pub tool: String,
    /// Optional argument pattern
    pub pattern: Option<String>,
}

impl std::fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.pattern {
            Some(p) => write!(f, "{}({})", self.tool, p),
            None => write!(f, "{}", self.tool),
        }
    }
}

impl PermissionRule {
    /// Parse `"Bash(cargo test)"` or `"Read"` into a `PermissionRule`.
    /// Returns `None` if the input is empty or malformed.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(paren) = s.find('(') {
            let tool = s[..paren].trim().to_lowercase();
            let rest = s[paren + 1..].trim_end_matches(')').trim().to_string();
            let pattern = if rest.is_empty() { None } else { Some(rest) };
            Some(Self { tool, pattern })
        } else {
            Some(Self {
                tool: s.to_lowercase(),
                pattern: None,
            })
        }
    }

    /// Returns `true` if this rule matches the given tool call.
    ///
    /// `tool_name` — the tool being invoked (e.g. "bash")
    /// `tool_arg`  — the first meaningful string argument (command / path)
    /// Canonical tool name (lowercase).
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Display string for the argument pattern, e.g. `(cargo test)` or empty.
    pub fn arg_display(&self) -> String {
        match &self.pattern {
            Some(p) => format!("({p})"),
            None => String::new(),
        }
    }

    pub fn matches(&self, tool_name: &str, tool_arg: Option<&str>) -> bool {
        if self.tool != tool_name.to_lowercase() {
            return false;
        }
        match (&self.pattern, tool_arg) {
            (None, _) => true,        // no pattern → match all invocations
            (Some(_), None) => false, // pattern requires an arg
            (Some(pat), Some(arg)) => pattern_matches(pat, arg),
        }
    }
}

/// Match `arg` against `pattern`.
///
/// Supported syntax:
///   `prefix:*`  — arg starts with `prefix` (`:*` is a suffix wildcard)
///   `prefix/**` — arg starts with `prefix/` (path glob wildcard)
///   `exact`     — exact equality
fn pattern_matches(pattern: &str, arg: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(":*") {
        // Command prefix wildcard: "rm -rf:*" matches "rm -rf foo"
        arg.starts_with(prefix)
    } else if let Some(prefix) = pattern.strip_suffix("/**") {
        // Path glob: "src/**" matches "src/foo/bar.rs"
        let dir = if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{prefix}/")
        };
        arg.starts_with(dir.as_str()) || arg == prefix
    } else if pattern == "**" {
        true
    } else {
        // Exact match (case-sensitive for paths/commands)
        arg == pattern
    }
}

/// Extract the first meaningful string argument from a tool's args JSON.
pub fn tool_first_arg(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    // Strip MCP server prefix (e.g. "desktop-commander__write_file" -> "write_file")
    let base_name = if let Some(pos) = tool_name.rfind("__") {
        &tool_name[pos + 2..]
    } else {
        tool_name
    };

    // Known arg key names per tool type
    let keys = match base_name.to_lowercase().as_str() {
        "bash" | "shell" | "run_command" | "execute_command" | "start_process" | "RunShellCommand" => {
            &["command", "cmd"][..]
        }
        "read_file" | "write_file" | "edit_file" | "create_file" | "delete_file" | "move_file"
        | "rename_file" | "apply_patch" | "edit_block" => &["path", "file_path", "filename"][..],
        _ => &["path", "command", "query"][..],
    };
    for key in keys {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

/// Permission mode controlling how tool calls are approved
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    /// Prompt user for each tool call (default)
    #[default]
    Default,
    /// Auto-allow Write/Edit file operations
    AcceptEdits,
    /// Observe & plan — tools run but state-mutating operations are blocked
    Plan,
    /// Allow all tools without prompting (--yolo)
    BypassPermissions,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::AcceptEdits => write!(f, "acceptEdits"),
            Self::Plan => write!(f, "plan"),
            Self::BypassPermissions => write!(f, "bypassPermissions"),
        }
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = crate::Error;
    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        match s {
            "default" => Ok(Self::Default),
            "acceptEdits" => Ok(Self::AcceptEdits),
            "plan" => Ok(Self::Plan),
            "bypassPermissions" => Ok(Self::BypassPermissions),
            other => Err(crate::Error::custom(format!(
                "unknown permission mode: {other}"
            ))),
        }
    }
}

// -- Verdict (unified permission resolution result)

/// The result of a single permission check via `PermissionManager::resolve()`.
///
/// Replaces the old trio of `is_blocked()` / `auto_approve()` / `block_reason()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Tool call may proceed without prompting the user.
    Allow,
    /// Tool call requires explicit user approval. Contains a human-readable reason.
    Ask(String),
    /// Tool call is hard-blocked and must NOT run. Contains a human-readable reason.
    Deny(String),
}

impl Verdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }
    pub fn is_ask(&self) -> bool {
        matches!(self, Self::Ask(_))
    }
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny(_))
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::Ask(r) | Self::Deny(r) => Some(r),
        }
    }
}

// -- Write-schema detection (schema-level filtering for Plan mode)

/// Returns true if the tool name represents a write/mutating operation.
/// Used to filter tool schemas out of the LLM's view in Plan mode.
pub fn is_write_schema(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "edit_file"
            | "create_file"
            | "delete_file"
            | "move_file"
            | "rename_file"
            | "patch_file"
            | "apply_patch"
            | "apply_diff"
            | "edit_block"
            | "desktop_control"
            | "send_notification"
    )
}

// -- Delete action detection

/// Returns true if a bash command's primary intent is file/directory deletion.
pub fn bash_first_cmd_is_delete(cmd: &str) -> bool {
    for segment in split_shell_segments(cmd) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let first = match tokens.first() {
            Some(t) => *t,
            None => continue,
        };
        let c = if first.contains('=') {
            tokens.get(1).copied().unwrap_or("")
        } else {
            first
        };
        if matches!(c, "rm" | "rmdir" | "unlink" | "shred") {
            return true;
        }
    }
    false
}

/// Returns true if the tool call represents a destructive delete action.
pub fn is_delete_action(
    tool_name: &str,
    base_name: &str,
    args: &serde_json::Value,
    is_mcp_write: bool,
) -> bool {
    // 1. Native tool name
    if base_name == "delete_file" {
        return true;
    }
    // 2. MCP tool — inspect full prefixed name for delete/remove keywords
    if is_mcp_write
        && (tool_name.contains("delete") || tool_name.contains("remove"))
    {
        return true;
    }
    // 3. Bash commands: rm, rmdir, unlink, shred
    if matches!(base_name, "bash") {
        let cmd = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return bash_first_cmd_is_delete(cmd);
    }
    false
}

// -- Write-tool and write-command detection

/// Shell commands (first token) that are considered read-only and always
/// permitted in plan mode when running via the `bash` tool.
const READONLY_CMDS: &[&str] = &[
    // filesystem observation
    "ls", "la", "ll", "dir", "tree", "cat", "less", "more", "head", "tail", "find", "fd", "locate",
    "file", "stat", "du", "df", "lsblk", // text search
    "grep", "rg", "ag", "awk", "sed", // sed without -i is read-only
    "wc", "sort", "uniq", "cut", "tr", "diff", "comm", "cmp", // path / env
    "pwd", "which", "whereis", "type", "echo", "printf", "date", "uname", "env", "printenv", "id",
    "whoami", "groups", "hostname", "uptime", // process / network observation
    "ps", "pgrep", "top", "htop", "lsof", "netstat", "ss", "ip", "ifconfig", "ping",
    // git — read-only subcommands handled separately
    "git",   // build inspection
    "cargo", // package observation
    "dpkg", "apt", "snap", "pip", "npm", "yarn",
];

/// Git subcommands that are read-only (all others are write).
const READONLY_GIT: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "tag",
    "remote",
    "stash", // stash list/show only — guarded by context check
    "describe",
    "shortlog",
    "reflog",
    "ls-files",
    "ls-tree",
    "cat-file",
    "config",
    "rev-parse",
    "rev-list",
    "blame",
    "grep",
    "bisect", // bisect start = write, but observation is fine
];

/// Cargo subcommands that are read-only.
const READONLY_CARGO: &[&str] = &[
    "check",
    "clippy",
    "test",
    "bench",
    "doc",
    "read-manifest",
    "locate-project",
    "metadata",
    "tree",
    "search",
    "info",
    "audit",
];

/// Returns true if the given path or command contains globally protected patterns
/// (.git/, .env, .ssh/) that should never be written to by the agent.
pub fn path_is_protected(path_or_cmd: &str) -> bool {
    let p = path_or_cmd.to_lowercase();
    // Normalize delimiters for boundary checking
    let norm = p.replace(
        |c: char| c.is_whitespace() || c == '=' || c == '"' || c == '\'' || c == '>',
        "/",
    );

    // Strip leading "./" and "../" sequences so relative paths like
    // "./.git" or "../.env" are correctly caught by starts_with checks.
    let stripped = {
        let mut s = norm.as_str();
        loop {
            if let Some(rest) = s.strip_prefix("./") {
                s = rest;
            } else if let Some(rest) = s.strip_prefix("../") {
                s = rest;
            } else {
                break s;
            }
        }
    };

    stripped.contains("/.git/")
        || stripped.starts_with(".git/")
        || stripped == ".git"
        || stripped.contains("/.ssh/")
        || stripped.starts_with(".ssh/")
        || stripped == ".ssh"
        || stripped.contains("/.env")
        || stripped.starts_with(".env")
        || stripped == ".env"
        || stripped.contains("/.cade-db")
        || stripped.starts_with(".cade-db")
        || stripped == ".cade-db"
}

/// Returns true if a bash `command` string would mutate the file system or
/// system state, making it inappropriate for plan mode.
///
/// Conservative: if the command cannot be determined to be read-only it is
/// treated as write (safe default).
pub fn bash_command_is_write(command: &str) -> bool {
    let cmd = command.trim();

    // Output redirection always writes
    if contains_write_redirect(cmd) {
        return true;
    }

    // Split on shell operators (;  &&  ||  |) and check each segment
    for segment in split_shell_segments(cmd) {
        if segment_is_write(segment.trim()) {
            return true;
        }
    }

    false
}

/// Returns true if the command contains high-risk patterns that should be flagged.
pub fn bash_command_is_suspicious(command: &str) -> bool {
    let cmd = command.to_lowercase();

    // 1. Nested shell execution / execution of arbitrary input
    let nested = [
        "$(", "`", "sh ", "bash ", "zsh ", "python ", "perl ", "php ", "ruby ", "node ",
    ];
    if nested.iter().any(|&p| cmd.contains(p)) {
        return true;
    }

    // 2. Suspicious network operations
    let network = ["curl", "wget", "nc ", "netcat", "ssh ", "telnet"];
    if network.iter().any(|&p| cmd.contains(p)) {
        return true;
    }

    // 3. Obfuscation attempts
    let obfuscation = ["base64", "hex", "xxd", "eval"];
    if obfuscation.iter().any(|&p| cmd.contains(p)) {
        return true;
    }

    // 4. Critical system files/dirs (if not just 'ls' or 'cat')
    let critical = ["/etc/passwd", "/etc/shadow", "/root/", "~/.ssh/", ".env"];
    if critical.iter().any(|&p| cmd.contains(p)) {
        return true;
    }

    false
}

fn contains_write_redirect(cmd: &str) -> bool {
    // Crude but effective: look for > that is not part of >>
    // and not inside a quoted string
    let chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    for c in chars {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '>' if !in_single && !in_double => return true, // > or >>
            _ => {}
        }
    }
    false
}

fn split_shell_segments(cmd: &str) -> Vec<&str> {
    // Split on ; && || and | (pipe) — very rough, good enough for safety
    let mut segments = Vec::new();
    let mut start = 0;
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b';' | b'|' | b'&' => {
                segments.push(&cmd[start..i]);
                // skip double operators (&&, ||, >>)
                if i + 1 < bytes.len()
                    && (bytes[i + 1] == b'&' || bytes[i + 1] == b'|' || bytes[i + 1] == b'>')
                {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    segments.push(&cmd[start..]);
    segments
}

fn segment_is_write(seg: &str) -> bool {
    let tokens: Vec<&str> = seg.split_whitespace().collect();
    let first = match tokens.first() {
        Some(t) => *t,
        None => return false,
    };

    // Strip leading env assignments like FOO=bar cmd …
    let cmd = if first.contains('=') {
        tokens.get(1).copied().unwrap_or("")
    } else {
        first
    };

    match cmd {
        // Not in the read-only list → treat as write (conservative)
        c if !READONLY_CMDS.contains(&c) => true,

        "git" => {
            let sub = tokens
                .get(if first.contains('=') { 2 } else { 1 })
                .copied()
                .unwrap_or("");
            // stash with arguments other than list/show is write
            if sub == "stash" {
                let action = tokens
                    .get(if first.contains('=') { 3 } else { 2 })
                    .copied()
                    .unwrap_or("list");
                return !matches!(action, "list" | "show");
            }
            !READONLY_GIT.contains(&sub)
        }

        "cargo" => {
            let sub = tokens
                .get(if first.contains('=') { 2 } else { 1 })
                .copied()
                .unwrap_or("");
            !READONLY_CARGO.contains(&sub)
        }

        // sed with -i modifies in-place
        "sed" => tokens
            .iter()
            .any(|t| t.starts_with("-i") || *t == "--in-place"),

        // awk, grep, diff, etc. are always read-only
        _ => false,
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests;

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
        let arg_ref = arg.as_deref();

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

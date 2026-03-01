use std::sync::{Arc, Mutex};

// ── PermissionRule ─────────────────────────────────────────────────────────────

/// A single allow or deny rule matching a tool call.
///
/// Syntax mirrors Letta Code:
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
            None    => write!(f, "{}", self.tool),
        }
    }
}

impl PermissionRule {
    /// Parse `"Bash(cargo test)"` or `"Read"` into a `PermissionRule`.
    /// Returns `None` if the input is empty or malformed.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() { return None; }
        if let Some(paren) = s.find('(') {
            let tool    = s[..paren].trim().to_lowercase();
            let rest    = s[paren + 1..].trim_end_matches(')').trim().to_string();
            let pattern = if rest.is_empty() { None } else { Some(rest) };
            Some(Self { tool, pattern })
        } else {
            Some(Self { tool: s.to_lowercase(), pattern: None })
        }
    }

    /// Returns `true` if this rule matches the given tool call.
    ///
    /// `tool_name` — the tool being invoked (e.g. "bash")
    /// `tool_arg`  — the first meaningful string argument (command / path)
    pub fn matches(&self, tool_name: &str, tool_arg: Option<&str>) -> bool {
        if self.tool != tool_name.to_lowercase() {
            return false;
        }
        match (&self.pattern, tool_arg) {
            (None, _)             => true,             // no pattern → match all invocations
            (Some(_), None)       => false,             // pattern requires an arg
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
        let dir = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
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
    // Known arg key names per tool type
    let keys = match tool_name.to_lowercase().as_str() {
        "bash" | "shell" | "run_command" | "execute_command"
            => &["command", "cmd"][..],
        "read_file" | "write_file" | "edit_file" | "create_file"
        | "delete_file" | "move_file" | "rename_file" | "apply_patch"
            => &["path", "file_path", "filename"][..],
        _   => &["path", "command", "query"][..],
    };
    for key in keys {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

/// Permission mode controlling how tool calls are approved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Prompt user for each tool call (default)
    Default,
    /// Auto-allow Write/Edit file operations
    AcceptEdits,
    /// Observe & plan — tools run but state-mutating operations are blocked
    Plan,
    /// Allow all tools without prompting (--yolo)
    BypassPermissions,
}

impl Default for PermissionMode {
    fn default() -> Self { Self::Default }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default           => write!(f, "default"),
            Self::AcceptEdits       => write!(f, "acceptEdits"),
            Self::Plan              => write!(f, "plan"),
            Self::BypassPermissions => write!(f, "bypassPermissions"),
        }
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "default"           => Ok(Self::Default),
            "acceptEdits"       => Ok(Self::AcceptEdits),
            "plan"              => Ok(Self::Plan),
            "bypassPermissions" => Ok(Self::BypassPermissions),
            other => anyhow::bail!("unknown permission mode: {other}"),
        }
    }
}

// ── Write-tool and write-command detection ────────────────────────────────────

/// Tools that are intrinsically write/mutating regardless of arguments.
/// In plan mode these are always blocked.
const WRITE_TOOLS: &[&str] = &[
    "write_file", "edit_file", "create_file", "delete_file",
    "move_file",  "rename_file", "patch_file", "apply_diff",
    "desktop_control",   // sends input / clicks
    "send_notification", // side-effect
];

/// Shell commands (first token) that are considered read-only and always
/// permitted in plan mode when running via the `bash` tool.
const READONLY_CMDS: &[&str] = &[
    // filesystem observation
    "ls", "la", "ll", "dir", "tree",
    "cat", "less", "more", "head", "tail",
    "find", "fd", "locate",
    "file", "stat", "du", "df", "lsblk",
    // text search
    "grep", "rg", "ag", "awk", "sed",  // sed without -i is read-only
    "wc", "sort", "uniq", "cut", "tr",
    "diff", "comm", "cmp",
    // path / env
    "pwd", "which", "whereis", "type",
    "echo", "printf", "date", "uname",
    "env", "printenv", "id", "whoami", "groups",
    "hostname", "uptime",
    // process / network observation
    "ps", "pgrep", "top", "htop", "lsof",
    "netstat", "ss", "ip", "ifconfig", "ping",
    // git — read-only subcommands handled separately
    "git",
    // build inspection
    "cargo",
    // package observation
    "dpkg", "apt", "snap", "pip", "npm", "yarn",
];

/// Git subcommands that are read-only (all others are write).
const READONLY_GIT: &[&str] = &[
    "status", "log", "diff", "show", "branch", "tag",
    "remote", "stash",  // stash list/show only — guarded by context check
    "describe", "shortlog", "reflog",
    "ls-files", "ls-tree", "cat-file",
    "config", "rev-parse", "rev-list",
    "blame", "grep", "bisect",  // bisect start = write, but observation is fine
];

/// Cargo subcommands that are read-only.
const READONLY_CARGO: &[&str] = &[
    "check", "clippy", "test", "bench",
    "doc", "read-manifest", "locate-project",
    "metadata", "tree", "search", "info",
    "audit",
];

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

fn contains_write_redirect(cmd: &str) -> bool {
    // Crude but effective: look for > that is not part of >>
    // and not inside a quoted string
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"'  if !in_single => in_double = !in_double,
            '>'  if !in_single && !in_double => return true, // > or >>
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
            let sub = tokens.get(if first.contains('=') { 2 } else { 1 }).copied().unwrap_or("");
            // stash with arguments other than list/show is write
            if sub == "stash" {
                let action = tokens.get(if first.contains('=') { 3 } else { 2 }).copied().unwrap_or("list");
                return !matches!(action, "list" | "show");
            }
            !READONLY_GIT.contains(&sub)
        }

        "cargo" => {
            let sub = tokens.get(if first.contains('=') { 2 } else { 1 }).copied().unwrap_or("");
            !READONLY_CARGO.contains(&sub)
        }

        // sed with -i modifies in-place
        "sed" => tokens.iter().any(|t| t.starts_with("-i") || *t == "--in-place"),

        // awk, grep, diff, etc. are always read-only
        _ => false,
    }
}

// ── PermissionManager ─────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct PermissionManager {
    mode:        Arc<Mutex<PermissionMode>>,
    allow_rules: Arc<Mutex<Vec<PermissionRule>>>,
    deny_rules:  Arc<Mutex<Vec<PermissionRule>>>,
}

impl PermissionManager {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode:        Arc::new(Mutex::new(mode)),
            allow_rules: Arc::new(Mutex::new(Vec::new())),
            deny_rules:  Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn mode(&self) -> PermissionMode { *self.mode.lock().unwrap() }
    pub fn set_mode(&self, mode: PermissionMode) { *self.mode.lock().unwrap() = mode; }

    pub fn add_allow_rule(&self, rule: PermissionRule) {
        let mut rules = self.allow_rules.lock().unwrap();
        if !rules.contains(&rule) { rules.push(rule); }
    }

    pub fn add_deny_rule(&self, rule: PermissionRule) {
        let mut rules = self.deny_rules.lock().unwrap();
        if !rules.contains(&rule) { rules.push(rule); }
    }

    pub fn allow_rules(&self) -> Vec<PermissionRule> { self.allow_rules.lock().unwrap().clone() }
    pub fn deny_rules(&self)  -> Vec<PermissionRule> { self.deny_rules.lock().unwrap().clone() }

    /// Returns true if the tool call should proceed without prompting.
    ///
    /// Resolution order (highest priority first):
    ///   1. deny_rules match  → NOT auto-approved (must prompt or block)
    ///   2. allow_rules match → auto-approved
    ///   3. Mode-based        → BypassPermissions=always, AcceptEdits=file writes
    pub fn auto_approve(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        let arg = tool_first_arg(tool_name, args);
        let arg_ref = arg.as_deref();

        // Explicit deny wins over everything
        if self.deny_rules.lock().unwrap().iter().any(|r| r.matches(tool_name, arg_ref)) {
            return false;
        }
        // Explicit allow
        if self.allow_rules.lock().unwrap().iter().any(|r| r.matches(tool_name, arg_ref)) {
            return true;
        }
        // Mode-based
        match self.mode() {
            PermissionMode::BypassPermissions => true,
            PermissionMode::AcceptEdits => {
                // File edits + apply_patch (Codex toolset)
                matches!(tool_name, "write_file" | "edit_file" | "apply_patch")
            }
            _ => false,
        }
    }

    /// Returns true if this tool call is blocked (must NOT run, even with approval).
    ///
    /// Resolution order:
    ///   1. deny_rules match in any mode → block
    ///   2. plan mode write detection    → block
    pub fn is_blocked(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        let arg = tool_first_arg(tool_name, args);
        let arg_ref = arg.as_deref();

        // Explicit deny rules block regardless of mode
        if self.deny_rules.lock().unwrap().iter().any(|r| r.matches(tool_name, arg_ref)) {
            return true;
        }

        if self.mode() != PermissionMode::Plan {
            return false;
        }

        // Plan mode: block write tools
        if WRITE_TOOLS.contains(&tool_name) {
            return true;
        }

        // Bash — allow read-only commands, block write ones
        if matches!(tool_name, "bash" | "shell" | "run_command" | "execute_command") {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            return bash_command_is_write(cmd);
        }

        false
    }

    /// Human-readable reason why a tool call was blocked.
    pub fn block_reason(&self, tool_name: &str, args: &serde_json::Value) -> String {
        let arg = tool_first_arg(tool_name, args);
        let arg_ref = arg.as_deref();

        // Check deny rule first
        if let Some(rule) = self.deny_rules.lock().unwrap().iter()
            .find(|r| r.matches(tool_name, arg_ref))
        {
            return format!("blocked by deny rule: {rule}");
        }

        if matches!(tool_name, "bash" | "shell" | "run_command" | "execute_command") {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            format!("plan mode: '{}' would modify system state", cmd.chars().take(60).collect::<String>())
        } else {
            format!("plan mode: '{tool_name}' is a write/mutating tool")
        }
    }
}

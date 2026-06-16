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
        "bash" | "shell" | "run_command" | "execute_command" | "start_process"
        | "RunShellCommand" => &["command", "cmd"][..],
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

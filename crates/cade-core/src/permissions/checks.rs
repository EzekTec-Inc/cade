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
        // P2-1: the canonical DB-key anchor moved to ~/.cade/db.key.
        // Protect both the directory and the file from agent writes.
        || stripped.contains("/.cade/db.key")
        || stripped.ends_with("/.cade/db.key")
        || stripped == ".cade/db.key"
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



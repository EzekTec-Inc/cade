// -- Write-schema detection (schema-level filtering for Plan mode & permission resolution)

/// Strip namespace and server prefixes (e.g. "default_api:serena__replace_content" -> "replace_content")
/// and convert to lower-case.
pub fn normalize_tool_name(name: &str) -> String {
    let mut s = name;
    if let Some(pos) = s.rfind(':') {
        s = &s[pos + 1..];
    }
    if let Some(pos) = s.rfind("__") {
        s = &s[pos + 2..];
    }
    s.trim().to_ascii_lowercase()
}

/// Returns true if the tool name represents a write/mutating/CRUD operation.
/// Used to enforce permission prompts and filter tool schemas in Plan mode.
pub fn is_write_schema(name: &str) -> bool {
    let clean = normalize_tool_name(name);

    // 0. Exclude internal agent meta-tools (memory, plan/todos, checkpoints, artifacts, reflection)
    // from requiring interactive permission prompts or being blocked in Plan mode.
    if matches!(
        clean.as_str(),
        "update_memory"
            | "update_memory_typed"
            | "update_memory_field"
            | "memory_apply_patch"
            | "archival_memory_insert"
            | "archival_memory_search"
            | "search_memory"
            | "conversation_search"
            | "query_event_log"
            | "recall"
            | "answer"
            | "link_memory_evidence"
            | "reflect"
            | "store_artifact"
            | "create_checkpoint"
            | "list_checkpoints"
            | "restore_checkpoint"
            | "update_plan"
            | "set_plan"
            | "todowrite"
            | "writetodos"
            | "finish_task"
            | "finishtask"
            | "enter_plan_mode"
            | "exit_plan_mode"
            | "enterplanmode"
            | "exitplanmode"
    ) {
        return false;
    }

    // 1. Exact matches across core CADE tools, Serena AST tools, GitHub MCP, and Desktop tools
    if matches!(
        clean.as_str(),
        "write_file"
            | "edit_file"
            | "create_file"
            | "create_text_file"
            | "create_directory"
            | "delete_file"
            | "delete_directory"
            | "move_file"
            | "rename_file"
            | "copy_file"
            | "patch_file"
            | "apply_patch"
            | "apply_diff"
            | "apply_edit"
            | "replace"
            | "replace_in_file"
            | "replace_content"
            | "replace_in_files"
            | "replace_symbol_body"
            | "insert_after_symbol"
            | "insert_before_symbol"
            | "rename_symbol"
            | "safe_delete_symbol"
            | "edit_block"
            | "desktop_control"
            | "send_notification"
            | "create_archive"
            | "extract_archive"
            | "create_issue"
            | "update_issue"
            | "add_issue_comment"
            | "create_pull_request"
            | "update_pull_request"
            | "merge_pull_request"
            | "create_branch"
            | "delete_branch"
            | "create_repository"
            | "delete_repository"
            | "create_or_update_file"
            | "trigger_workflow"
            | "launch_app"
            | "close_window"
            | "lock_screen"
            | "set_theme"
            | "set_wallpaper"
            | "set_volume"
            | "set_brightness"
            | "kill_process"
            | "kill_session"
            | "kill_sessions"
            | "clipboard_write"
            | "write_clipboard"
            | "install_plugin"
            | "install_skill"
            | "lql_insert"
            | "lql_delete"
            | "lql_apply_patch"
    ) {
        return true;
    }

    // 2. Semantic prefix stems for CRUD and mutating tools
    let stems = [
        "write_", "create_", "edit_", "replace_", "insert_", "delete_", "remove_", "update_",
        "patch_", "rename_", "copy_", "move_", "kill_", "apply_", "set_",
    ];
    stems.iter().any(|stem| clean.starts_with(stem))
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
    let clean = normalize_tool_name(base_name);
    let clean_full = normalize_tool_name(tool_name);

    // 1. Native and AST deletion tools
    if matches!(
        clean.as_str(),
        "delete_file"
            | "delete_directory"
            | "safe_delete_symbol"
            | "delete_branch"
            | "delete_repository"
            | "kill_process"
            | "kill_session"
            | "kill_sessions"
            | "lql_delete"
    ) {
        return true;
    }

    // 2. Generic delete/remove/drop/destroy naming
    if clean.starts_with("delete_")
        || clean.starts_with("remove_")
        || clean.starts_with("drop_")
        || clean.starts_with("destroy_")
        || clean.starts_with("kill_")
    {
        return true;
    }

    // 3. MCP tool — inspect full prefixed name for delete/remove/drop/destroy keywords
    if is_mcp_write
        && (clean_full.contains("delete")
            || clean_full.contains("remove")
            || clean_full.contains("drop")
            || clean_full.contains("destroy")
            || clean_full.contains("kill"))
    {
        return true;
    }

    // 4. Bash commands: rm, rmdir, unlink, shred
    if matches!(clean.as_str(), "bash" | "runshellcommand" | "shell") {
        let cmd = args
            .get("command")
            .or_else(|| args.get("cmd"))
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
    let p = path_or_cmd.to_lowercase().replace('\\', "/");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_write_schema_recognizes_all_crud_and_mutating_tools() {
        // Core CADE editing and creation tools
        assert!(is_write_schema("write_file"));
        assert!(is_write_schema("edit_file"));
        assert!(is_write_schema("create_file"));
        assert!(is_write_schema("Replace"));
        assert!(is_write_schema("replace_in_file"));
        assert!(is_write_schema("edit_block"));
        assert!(is_write_schema("apply_patch"));

        // Serena AST mutation tools with and without namespaces
        assert!(is_write_schema("serena__replace_content"));
        assert!(is_write_schema("serena__create_text_file"));
        assert!(is_write_schema("serena__replace_symbol_body"));
        assert!(is_write_schema("serena__insert_after_symbol"));
        assert!(is_write_schema("serena__insert_before_symbol"));
        assert!(is_write_schema("serena__rename_symbol"));
        assert!(is_write_schema("serena__safe_delete_symbol"));
        assert!(is_write_schema("default_api:serena__replace_content"));

        // Desktop Commander mutation tools
        assert!(is_write_schema("desktop-commander-mcp__write_file"));
        assert!(is_write_schema("desktop-commander-mcp__delete_file"));
        assert!(is_write_schema("desktop-commander-mcp__edit_block"));
        assert!(is_write_schema("desktop-commander-mcp__create_directory"));
        assert!(is_write_schema("desktop-commander-mcp__kill_process"));

        // GitHub mutating operations
        assert!(is_write_schema("github-mcp-server__create_issue"));
        assert!(is_write_schema("github-mcp-server__update_pull_request"));
        assert!(is_write_schema("github-mcp-server__merge_pull_request"));
        assert!(is_write_schema("github-mcp-server__create_branch"));
        assert!(is_write_schema("github-mcp-server__delete_branch"));

        // Read-only tools must NOT be classified as write schemas
        assert!(!is_write_schema("read_file"));
        assert!(!is_write_schema("glob"));
        assert!(!is_write_schema("grep"));
        assert!(!is_write_schema("fetch_doc"));
        assert!(!is_write_schema("serena__find_symbol"));
        assert!(!is_write_schema("serena__read_file"));
        assert!(!is_write_schema("serena__search_for_pattern"));
        assert!(!is_write_schema("desktop-commander-mcp__read_file"));
        assert!(!is_write_schema("github-mcp-server__get_pull_request"));
    }

    #[test]
    fn test_is_delete_action_recognizes_destructive_operations() {
        assert!(is_delete_action(
            "delete_file",
            "delete_file",
            &json!({}),
            false
        ));
        assert!(is_delete_action(
            "safe_delete_symbol",
            "safe_delete_symbol",
            &json!({}),
            false
        ));
        assert!(is_delete_action(
            "serena__safe_delete_symbol",
            "safe_delete_symbol",
            &json!({}),
            true
        ));
        assert!(is_delete_action(
            "desktop-commander-mcp__delete_file",
            "delete_file",
            &json!({}),
            true
        ));
        assert!(is_delete_action(
            "github-mcp-server__delete_branch",
            "delete_branch",
            &json!({}),
            true
        ));
        assert!(is_delete_action(
            "kill_process",
            "kill_process",
            &json!({}),
            false
        ));

        // Shell delete commands
        assert!(is_delete_action(
            "bash",
            "bash",
            &json!({"command": "rm -rf /tmp/target"}),
            false
        ));
        assert!(is_delete_action(
            "RunShellCommand",
            "RunShellCommand",
            &json!({"command": "rmdir foo"}),
            false
        ));
        assert!(is_delete_action(
            "shell",
            "shell",
            &json!({"command": "shred file.txt"}),
            false
        ));

        // Non-delete write operations must return false for is_delete_action
        assert!(!is_delete_action(
            "write_file",
            "write_file",
            &json!({}),
            false
        ));
        assert!(!is_delete_action("Replace", "Replace", &json!({}), false));
        assert!(!is_delete_action(
            "serena__replace_content",
            "replace_content",
            &json!({}),
            true
        ));
        assert!(!is_delete_action(
            "bash",
            "bash",
            &json!({"command": "cargo build"}),
            false
        ));
    }
}

// endregion: --- Tests

/// Nerd Font icon mapping for tool calls and UI elements.
///
/// Each function returns a Nerd Font glyph when `nerd` is true,
/// or a plain ASCII/Unicode fallback when false.
///
/// Icon codepoints from Nerd Fonts v3 — requires a patched font
/// (e.g. JetBrainsMono Nerd Font, FiraCode Nerd Font).

// region:    --- Tool icons

/// Return a glyph for the given tool name (post `display_tool_name` stripping).
/// Falls back to a generic tool icon for unrecognised names.
pub fn tool_icon(stripped_name: &str, nerd: bool) -> &'static str {
    if !nerd {
        return "▶";
    }
    match stripped_name {
        // -- Shell / process
        "bash" | "shell" | "run_command" | "execute_command" | "start_process"
        | "RunShellCommand" => "\u{f120}", //

        // -- File read
        "read_file" | "ReadFileGemini" | "read_multiple_files" => "\u{f15c}", //

        // -- File write / edit
        "write_file" | "edit_file" | "create_file" | "edit_block" | "replace_in_file" => "\u{f0f6}", //

        // -- Patch / diff
        "apply_patch" | "ide_apply_patch" => "\u{f440}", //

        // -- Search / grep
        "grep" | "grep_search" | "GlobGemini" | "SearchFileContent" | "start_search"
        | "find_references" | "symbol_search" => "\u{f002}", //

        // -- Directory / glob
        "list_directory" | "glob" | "get_file_info" => "\u{f07b}", //

        // -- Git
        "commit" | "push" | "pull" | "branch" | "merge" | "rebase_op" | "stash_op" | "log"
        | "diff" | "status" | "add" | "reset" | "restore" | "fetch" | "remote" | "tag" | "show"
        | "blame" | "cherry_pick" | "clean" | "revert" | "config" | "repository" => "\u{e725}", //

        // -- GitHub
        "create_pull_request"
        | "create_issue"
        | "list_issues"
        | "search_issues"
        | "search_code"
        | "get_issue"
        | "add_issue_comment"
        | "list_commits"
        | "get_file_contents"
        | "get_repository"
        | "create_branch"
        | "search_repositories"
        | "update_issue"
        | "get_user" => "\u{f09b}", //

        // -- Memory / knowledge
        "update_memory"
        | "memory_apply_patch"
        | "search_memory"
        | "conversation_search"
        | "archival_memory_insert"
        | "archival_memory_search"
        | "update_memory_typed"
        | "link_memory_evidence"
        | "reflect" => "\u{f0eb}", //

        // -- Skills
        "load_skill" | "install_skill" | "run_skill_script" | "load_skill_ref" => "\u{f085}", //

        // -- Subagents
        "run_subagent" | "list_agents" | "message_agent" => "\u{f0c0}", //

        // -- Plan / task
        "EnterPlanMode" | "ExitPlanMode" | "TodoWrite" | "UpdatePlan" | "WriteTodos"
        | "set_plan" | "workflow" => "\u{f0ae}", //

        // -- Checkpoints / artifacts
        "create_checkpoint" | "restore_checkpoint" | "list_checkpoints" | "store_artifact" => {
            "\u{f0c7}"
        } //

        // -- Web / network
        "web_search" | "fetch_doc" | "browser_screenshot" | "http_request" | "get-library-docs"
        | "resolve-library-id" => "\u{f0ac}", //

        // -- Desktop
        "screen_capture"
        | "desktop_screenshot"
        | "list_windows"
        | "desktop_list_windows"
        | "desktop_control"
        | "image_processor" => "\u{f108}", //

        // -- Clipboard
        "clipboard_read" | "clipboard_write" => "\u{f328}", //

        // -- Question / interaction
        "ask_user_question" => "\u{f128}", //

        // -- LSP
        "get_definition" | "get_references" => "\u{f121}", //

        // -- Format
        "format_code" => "\u{f0d0}", //

        // -- OpenViking
        "find_viking" | "grep_viking" | "ls_viking" | "tree_viking" => "\u{f0ac}", //

        // -- Neovim IDE
        "ide_read_buffer" | "ide_propose_edit" => "\u{f36f}", // (neovim)

        // -- Default
        _ => "\u{f0ad}", //  (wrench — generic tool)
    }
}

// endregion: --- Tool icons

// region:    --- Status icons

/// Icon for a successful tool result.
pub fn success_icon(nerd: bool) -> &'static str {
    if nerd { "\u{f058}" } else { "✓" } //  vs ✓
}

/// Icon for a failed tool result.
pub fn error_icon(nerd: bool) -> &'static str {
    if nerd { "\u{f057}" } else { "✗" } //  vs ✗
}

// endregion: --- Status icons

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tool_returns_specific_icon() {
        assert_eq!(tool_icon("bash", true), "\u{f120}");
        assert_eq!(tool_icon("read_file", true), "\u{f15c}");
        assert_eq!(tool_icon("commit", true), "\u{e725}");
        assert_eq!(tool_icon("create_pull_request", true), "\u{f09b}");
        assert_eq!(tool_icon("update_memory", true), "\u{f0eb}");
    }

    #[test]
    fn unknown_tool_returns_default_icon() {
        assert_eq!(tool_icon("some_random_tool", true), "\u{f0ad}");
    }

    #[test]
    fn nerd_disabled_always_returns_ascii_fallback() {
        assert_eq!(tool_icon("bash", false), "▶");
        assert_eq!(tool_icon("read_file", false), "▶");
        assert_eq!(tool_icon("some_random_tool", false), "▶");
    }

    #[test]
    fn status_icons_nerd_mode() {
        assert_eq!(success_icon(true), "\u{f058}");
        assert_eq!(error_icon(true), "\u{f057}");
    }

    #[test]
    fn status_icons_ascii_fallback() {
        assert_eq!(success_icon(false), "✓");
        assert_eq!(error_icon(false), "✗");
    }
}

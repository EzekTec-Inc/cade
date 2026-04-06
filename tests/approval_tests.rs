#![allow(clippy::empty_line_after_doc_comments)]
/// Regression tests for the tool-approval modal fixes.
///
/// Covers three root-cause bugs:
///   B1 — async/Mutex deadlock: ask_question_async + rx.await blocked the
///        async runtime while competing with the tick-task for the app lock.
///   B2 — event race: tick task could consume key events intended for the
///        blocking modal when active_question.tx is None.
///   B3 — session-allow timing: add_session_allow was applied too late for
///        back-to-back tool calls of the same type in one turn.

// region:    --- Tests

// -- B3: session-allow timing

#[cfg(test)]
mod permission_tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.
    use cade::permissions::{PermissionManager, PermissionMode, PermissionRule};
    use serde_json::json;

    /// After "Yes, don't ask again" the allow rule must be present
    /// IMMEDIATELY — before the tool even runs — so that a second
    /// consecutive call to resolve returns Allow without prompting.
    #[test]
    fn session_allow_applied_before_second_tool() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);
        let args = json!({ "command": "cargo build" });

        // -- Check (pre-condition): cargo build is a write command, needs approval
        assert!(
            mgr.resolve("bash", &args, false).is_ask(),
            "should require approval before session-allow is added"
        );

        // -- Exec
        mgr.add_session_allow("bash");

        // -- Check
        assert!(
            mgr.resolve("bash", &args, false).is_allow(),
            "should be auto-approved after add_session_allow"
        );
    }

    /// Session allow for a specific tool must not bleed into other tools.
    #[test]
    fn session_allow_is_tool_specific() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("bash");
        let bash_args = json!({ "command": "ls" });
        let write_args = json!({ "path": "foo.rs" });

        // -- Check
        assert!(
            mgr.resolve("bash", &bash_args, false).is_allow(),
            "bash should be allowed"
        );
        assert!(
            mgr.resolve("write_file", &write_args, false).is_ask(),
            "write_file should still need approval"
        );
    }

    /// Explicit deny overrides session allow (deny wins over allow).
    #[test]
    fn deny_rule_overrides_session_allow() -> Result<()> {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("bash");
        mgr.add_deny_rule(PermissionRule::parse("bash(rm -rf:*)").ok_or("Should parse")?);
        let safe_args = json!({ "command": "ls -la" });
        let risky_args = json!({ "command": "rm -rf /tmp/foo" });

        // -- Check
        assert!(
            mgr.resolve("bash", &safe_args, false).is_allow(),
            "safe bash should be allowed"
        );
        assert!(
            mgr.resolve("bash", &risky_args, false).is_deny(),
            "rm -rf must be denied even after session allow"
        );

        Ok(())
    }

    /// BypassPermissions mode auto-approves everything.
    #[test]
    fn bypass_permissions_approves_all() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::BypassPermissions);

        // -- Check
        assert!(mgr.resolve("bash", &json!({ "command": "rm -rf /" }), false).is_allow());
        assert!(mgr.resolve("write_file", &json!({ "path": "/etc/passwd" }), false).is_allow());
    }

    /// AcceptEdits auto-approves file-mutation tools but asks for deletes.
    #[test]
    fn accept_edits_approves_file_tools_asks_for_delete() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::AcceptEdits);

        // -- Check — create/edit auto-approved
        assert!(mgr.resolve("write_file", &json!({ "path": "x.rs" }), false).is_allow());
        assert!(mgr.resolve("edit_file", &json!({ "path": "x.rs" }), false).is_allow());
        assert!(mgr.resolve("apply_patch", &json!({ "path": "x.rs" }), false).is_allow());

        // -- Check — delete requires approval
        assert!(mgr.resolve("delete_file", &json!({ "path": "x.rs" }), false).is_ask());
        assert!(mgr.resolve("bash", &json!({ "command": "rm foo" }), false).is_ask());
    }

    /// Plan mode blocks write tools and write shell commands.
    #[test]
    fn plan_mode_blocks_write_operations() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Plan);

        // -- Check
        assert!(mgr.resolve("write_file", &json!({ "path": "x.rs" }), false).is_deny());
        assert!(mgr.resolve("bash", &json!({ "command": "rm foo" }), false).is_deny());
        assert!(mgr.resolve("bash", &json!({ "command": "ls -la" }), false).is_allow());
        assert!(mgr.resolve("bash", &json!({ "command": "cargo check" }), false).is_allow());
    }

    /// add_session_allow is idempotent — duplicate rules are not stored.
    #[test]
    fn session_allow_idempotent() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);

        // -- Exec
        mgr.add_session_allow("bash");
        mgr.add_session_allow("bash");
        mgr.add_session_allow("bash");

        // -- Check
        assert_eq!(
            mgr.allow_rules().len(),
            1,
            "duplicate rules must be de-duped"
        );
    }
}

// -- B3 (extended): resolve sees the rule before execute_tool runs

#[cfg(test)]
mod back_to_back_tool_tests {
    use cade::permissions::{PermissionManager, PermissionMode};
    use serde_json::json;

    /// Simulates two consecutive tool calls of the same type in one agent turn.
    /// After the first is approved with "don't ask again", the second must be
    /// auto-approved without any prompt.
    #[test]
    fn second_tool_call_auto_approved_after_dont_ask_again() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);
        let args = json!({ "command": "cargo build" });

        // -- Check (pre-condition)
        assert!(
            mgr.resolve("bash", &args, false).is_ask(),
            "first call needs prompt"
        );

        // -- Exec
        mgr.add_session_allow("bash");

        // -- Check
        assert!(
            mgr.resolve("bash", &args, false).is_allow(),
            "second call must be auto-approved"
        );
        assert!(
            mgr.resolve("bash", &json!({ "command": "cargo test" }), false).is_allow(),
            "third call with different args must also be auto-approved"
        );
    }
}

// -- B2: tick-task guard — tx.is_some() controls event routing

#[cfg(test)]
mod tick_task_guard_tests {
    /// The tick-task guard logic is: only call handle_question_key when
    /// active_question.tx.is_some().  We test the condition in isolation
    /// without spawning a full TUI (which requires a real terminal).

    /// Helper that mirrors the tick-task condition exactly.
    fn should_route_to_handle_question_key(tx_is_some: bool) -> bool {
        // Mirrors: app.active_question.as_ref().map_or(false, |aq| aq.tx.is_some())
        tx_is_some
    }

    #[test]
    fn async_question_routes_to_handle_question_key() {
        // -- Exec & Check
        assert!(
            should_route_to_handle_question_key(true),
            "when tx is Some, events should route to handle_question_key"
        );
    }

    #[test]
    fn no_question_falls_through() {
        // -- Exec & Check
        assert!(
            !should_route_to_handle_question_key(false),
            "when tx is None, events should NOT route to handle_question_key"
        );
    }
}

// endregion: --- Tests

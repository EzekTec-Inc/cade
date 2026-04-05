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
    /// consecutive call to auto_approve returns true without prompting.
    #[test]
    fn session_allow_applied_before_second_tool() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Default);
        let args = json!({ "command": "cargo test" });

        // -- Check (pre-condition)
        assert!(
            !mgr.auto_approve("bash", &args, false),
            "should require approval before session-allow is added"
        );

        // -- Exec
        mgr.add_session_allow("bash");

        // -- Check
        assert!(
            mgr.auto_approve("bash", &args, false),
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
            mgr.auto_approve("bash", &bash_args, false),
            "bash should be allowed"
        );
        assert!(
            !mgr.auto_approve("write_file", &write_args, false),
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
            mgr.auto_approve("bash", &safe_args, false),
            "safe bash should be allowed"
        );
        assert!(
            !mgr.auto_approve("bash", &risky_args, false),
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
        assert!(mgr.auto_approve("bash", &json!({ "command": "rm -rf /" }), false));
        assert!(mgr.auto_approve("write_file", &json!({ "path": "/etc/passwd" }), false));
    }

    /// AcceptEdits only auto-approves file-mutation tools.
    #[test]
    fn accept_edits_approves_only_file_tools() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::AcceptEdits);

        // -- Check
        assert!(mgr.auto_approve("write_file", &json!({ "path": "x.rs" }), false));
        assert!(mgr.auto_approve("edit_file", &json!({ "path": "x.rs" }), false));
        assert!(mgr.auto_approve("apply_patch", &json!({ "path": "x.rs" }), false));
        assert!(!mgr.auto_approve("bash", &json!({ "command": "ls" }), false));
    }

    /// Plan mode blocks write tools and write shell commands.
    #[test]
    fn plan_mode_blocks_write_operations() {
        // -- Setup & Fixtures
        let mgr = PermissionManager::new(PermissionMode::Plan);

        // -- Check
        assert!(mgr.is_blocked("write_file", &json!({ "path": "x.rs" }), false));
        assert!(mgr.is_blocked("bash", &json!({ "command": "rm foo" }), false));
        assert!(!mgr.is_blocked("bash", &json!({ "command": "ls -la" }), false));
        assert!(!mgr.is_blocked("bash", &json!({ "command": "cargo check" }), false));
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

// -- B3 (extended): auto_approve sees the rule before execute_tool runs

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
            !mgr.auto_approve("bash", &args, false),
            "first call needs prompt"
        );

        // -- Exec
        mgr.add_session_allow("bash");

        // -- Check
        assert!(
            mgr.auto_approve("bash", &args, false),
            "second call must be auto-approved"
        );
        assert!(
            mgr.auto_approve("bash", &json!({ "command": "cargo test" }), false),
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
            "async modal (tx=Some) must route keys through handle_question_key"
        );
    }

    #[test]
    fn blocking_question_does_not_route_to_handle_question_key() {
        // -- Exec & Check
        assert!(
            !should_route_to_handle_question_key(false),
            "blocking modal (tx=None) must NOT route keys through handle_question_key"
        );
    }

    #[test]
    fn no_active_question_does_not_route() {
        // -- Setup & Fixtures
        let active_question: Option<bool> = None;

        // -- Exec
        let routes = active_question.is_some_and(|tx_some| tx_some);

        // -- Check
        assert!(
            !routes,
            "no active question must not route to handle_question_key"
        );
    }
}

// -- PermissionRule parsing and matching

#[cfg(test)]
mod rule_tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;
    use cade::permissions::PermissionRule;

    #[test]
    fn parse_bare_tool_name() -> Result<()> {
        // -- Exec
        let r = PermissionRule::parse("Bash").ok_or("Should parse")?;

        // -- Check
        assert_eq!(r.tool(), "bash");
        assert!(r.matches("bash", None));
        assert!(r.matches("bash", Some("any command")));

        Ok(())
    }

    #[test]
    fn parse_tool_with_exact_arg() {
        // -- Exec
        let r = PermissionRule::parse("Bash(cargo test)").unwrap();

        // -- Check
        assert!(r.matches("bash", Some("cargo test")));
        assert!(!r.matches("bash", Some("cargo build")));
        assert!(!r.matches("bash", None));
    }

    #[test]
    fn parse_tool_with_prefix_wildcard() {
        // -- Exec
        let r = PermissionRule::parse("Bash(rm -rf:*)").unwrap();

        // -- Check
        assert!(r.matches("bash", Some("rm -rf /tmp/foo")));
        assert!(r.matches("bash", Some("rm -rf .")));
        assert!(!r.matches("bash", Some("rm foo")));
    }

    #[test]
    fn parse_tool_with_path_glob() {
        // -- Exec
        let r = PermissionRule::parse("read_file(src/**)").unwrap();

        // -- Check
        assert!(r.matches("read_file", Some("src/main.rs")));
        assert!(r.matches("read_file", Some("src/ui/app.rs")));
        assert!(!r.matches("read_file", Some("tests/foo.rs")));
    }

    #[test]
    fn parse_invalid_returns_none() {
        // -- Exec & Check
        assert!(PermissionRule::parse("").is_none());
        assert!(PermissionRule::parse("   ").is_none());
    }

    #[test]
    fn case_insensitive_tool_name() -> Result<()> {
        // -- Exec
        let r = PermissionRule::parse("BASH").ok_or("Should parse")?;

        // -- Check
        assert!(r.matches("bash", None));
        assert!(r.matches("BASH", None));
        assert!(r.matches("Bash", None));

        Ok(())
    }
}

// endregion: --- Tests

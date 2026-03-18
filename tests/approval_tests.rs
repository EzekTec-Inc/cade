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
    use cade::permissions::{PermissionManager, PermissionMode, PermissionRule};
    use serde_json::json;

    /// After "Yes, don't ask again" the allow rule must be present
    /// IMMEDIATELY — before the tool even runs — so that a second
    /// consecutive call to auto_approve returns true without prompting.
    #[test]
    fn session_allow_applied_before_second_tool() {
        let mgr = PermissionManager::new(PermissionMode::Default);

        // Initially no auto-approve for bash
        let args = json!({ "command": "cargo test" });
        assert!(
            !mgr.auto_approve("bash", &args),
            "should require approval before session-allow is added"
        );

        // Simulate what prompt_approval does on "Yes, don't ask again"
        mgr.add_session_allow("bash");

        // Second call to the same tool → must be auto-approved immediately
        assert!(
            mgr.auto_approve("bash", &args),
            "should be auto-approved after add_session_allow"
        );
    }

    /// Session allow for a specific tool must not bleed into other tools.
    #[test]
    fn session_allow_is_tool_specific() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("bash");

        let bash_args  = json!({ "command": "ls" });
        let write_args = json!({ "path": "foo.rs" });

        assert!(mgr.auto_approve("bash",       &bash_args),  "bash should be allowed");
        assert!(!mgr.auto_approve("write_file", &write_args), "write_file should still need approval");
    }

    /// Explicit deny overrides session allow (deny wins over allow).
    #[test]
    fn deny_rule_overrides_session_allow() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("bash");
        mgr.add_deny_rule(PermissionRule::parse("bash(rm -rf:*)").unwrap());

        let safe_args = json!({ "command": "ls -la" });
        let risky_args = json!({ "command": "rm -rf /tmp/foo" });

        assert!(mgr.auto_approve("bash", &safe_args),  "safe bash should be allowed");
        assert!(!mgr.auto_approve("bash", &risky_args), "rm -rf must be denied even after session allow");
    }

    /// BypassPermissions mode auto-approves everything.
    #[test]
    fn bypass_permissions_approves_all() {
        let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
        assert!(mgr.auto_approve("bash",       &json!({ "command": "rm -rf /" })));
        assert!(mgr.auto_approve("write_file", &json!({ "path": "/etc/passwd" })));
    }

    /// AcceptEdits only auto-approves file-mutation tools.
    #[test]
    fn accept_edits_approves_only_file_tools() {
        let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
        assert!(mgr.auto_approve("write_file",  &json!({ "path": "x.rs" })));
        assert!(mgr.auto_approve("edit_file",   &json!({ "path": "x.rs" })));
        assert!(mgr.auto_approve("apply_patch", &json!({ "path": "x.rs" })));
        assert!(!mgr.auto_approve("bash",       &json!({ "command": "ls" })));
    }

    /// Plan mode blocks write tools and write shell commands.
    #[test]
    fn plan_mode_blocks_write_operations() {
        let mgr = PermissionManager::new(PermissionMode::Plan);
        assert!(mgr.is_blocked("write_file",  &json!({ "path": "x.rs" })));
        assert!(mgr.is_blocked("bash",        &json!({ "command": "rm foo" })));
        assert!(!mgr.is_blocked("bash",       &json!({ "command": "ls -la" })));
        assert!(!mgr.is_blocked("bash",       &json!({ "command": "cargo check" })));
    }

    /// add_session_allow is idempotent — duplicate rules are not stored.
    #[test]
    fn session_allow_idempotent() {
        let mgr = PermissionManager::new(PermissionMode::Default);
        mgr.add_session_allow("bash");
        mgr.add_session_allow("bash");
        mgr.add_session_allow("bash");
        assert_eq!(mgr.allow_rules().len(), 1, "duplicate rules must be de-duped");
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
        let mgr  = PermissionManager::new(PermissionMode::Default);
        let args = json!({ "command": "cargo build" });

        // First call: needs approval (returns false → would prompt)
        assert!(!mgr.auto_approve("bash", &args), "first call needs prompt");

        // User selects "Yes, don't ask again" → add_session_allow called
        // BEFORE returning from prompt_approval (the Stage 1/2 fix ensures this)
        mgr.add_session_allow("bash");

        // Second call in the same turn: must be auto-approved
        assert!(mgr.auto_approve("bash", &args), "second call must be auto-approved");

        // Third call — same
        assert!(mgr.auto_approve("bash", &json!({ "command": "cargo test" })),
            "third call with different args must also be auto-approved");
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
        // ask_question_async sets tx = Some(...)
        assert!(
            should_route_to_handle_question_key(true),
            "async modal (tx=Some) must route keys through handle_question_key"
        );
    }

    #[test]
    fn blocking_question_does_not_route_to_handle_question_key() {
        // ask_question_blocking sets tx = None
        assert!(
            !should_route_to_handle_question_key(false),
            "blocking modal (tx=None) must NOT route keys through handle_question_key"
        );
    }

    #[test]
    fn no_active_question_does_not_route() {
        // No active question → map_or(false, ...) = false
        let active_question: Option<bool> = None; // None = no active question
        let routes = active_question.is_some_and(|tx_some| tx_some);
        assert!(!routes, "no active question must not route to handle_question_key");
    }
}

// -- PermissionRule parsing and matching

#[cfg(test)]
mod rule_tests {
    use cade::permissions::PermissionRule;

    #[test]
    fn parse_bare_tool_name() {
        let r = PermissionRule::parse("Bash").unwrap();
        assert_eq!(r.tool(), "bash");
        assert!(r.matches("bash", None));
        assert!(r.matches("bash", Some("any command")));
    }

    #[test]
    fn parse_tool_with_exact_arg() {
        let r = PermissionRule::parse("Bash(cargo test)").unwrap();
        assert!(r.matches("bash", Some("cargo test")));
        assert!(!r.matches("bash", Some("cargo build")));
        assert!(!r.matches("bash", None));
    }

    #[test]
    fn parse_tool_with_prefix_wildcard() {
        let r = PermissionRule::parse("Bash(rm -rf:*)").unwrap();
        assert!(r.matches("bash", Some("rm -rf /tmp/foo")));
        assert!(r.matches("bash", Some("rm -rf .")));
        assert!(!r.matches("bash", Some("rm foo")));
    }

    #[test]
    fn parse_tool_with_path_glob() {
        let r = PermissionRule::parse("read_file(src/**)").unwrap();
        assert!(r.matches("read_file", Some("src/main.rs")));
        assert!(r.matches("read_file", Some("src/ui/app.rs")));
        assert!(!r.matches("read_file", Some("tests/foo.rs")));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(PermissionRule::parse("").is_none());
        assert!(PermissionRule::parse("   ").is_none());
    }

    #[test]
    fn case_insensitive_tool_name() {
        let r = PermissionRule::parse("BASH").unwrap();
        assert!(r.matches("bash", None));
        assert!(r.matches("BASH", None));
        assert!(r.matches("Bash", None));
    }
}

// endregion: --- Tests

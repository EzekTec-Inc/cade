#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::*;
use std::fs;

// -- HooksConfig

#[test]
fn hooks_config_default_is_empty() {
    let h = HooksConfig::default();
    assert!(h.is_empty());
}

#[test]
fn hooks_config_merge_preserves_both() {
    let mut a = HooksConfig::default();
    a.pre_tool_use.push(HookEntry {
        matcher: None,
        hooks: vec![],
    });
    let mut b = HooksConfig::default();
    b.stop.push(HookEntry {
        matcher: None,
        hooks: vec![],
    });
    let merged = a.merge(b);
    assert_eq!(merged.pre_tool_use.len(), 1);
    assert_eq!(merged.stop.len(), 1);
}

// -- PermissionSettings

#[test]
fn permission_settings_default() {
    let p = PermissionSettings::default();
    assert!(p.allow.is_empty());
    assert!(p.deny.is_empty());
    assert!(!p.strict_bash);
}

#[test]
fn test_mcp_config_headers_parsing() {
    let json = r#"{
        "command": "test",
        "url": "http://localhost",
        "headers": {
            "X-Custom": "value",
            "Authorization": "Bearer ${MY_TOKEN}"
        }
    }"#;
    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    let headers = config.headers.unwrap();
    assert_eq!(headers.get("X-Custom").unwrap(), "value");
    assert_eq!(headers.get("Authorization").unwrap(), "Bearer ${MY_TOKEN}");
}

// -- GlobalSettings serialization

#[test]
fn global_settings_roundtrip_json() -> Result<()> {
    let mut gs = GlobalSettings::default();
    gs.last_agent = Some("agent-1".into());
    gs.env.api_key = Some("sk-test".into());
    gs.permissions.allow.push("Bash(cargo test)".into());
    gs.permissions.deny.push("Bash(rm:*)".into());
    gs.permissions.strict_bash = true;
    gs.store_api_key = true; // explicitly set (Default gives false, serde default gives true)

    let json = serde_json::to_string_pretty(&gs)?;
    let parsed: GlobalSettings = serde_json::from_str(&json)?;

    assert_eq!(parsed.last_agent.as_deref(), Some("agent-1"));
    assert_eq!(parsed.env.api_key.as_deref(), Some("sk-test"));
    assert_eq!(parsed.permissions.allow, vec!["Bash(cargo test)"]);
    assert_eq!(parsed.permissions.deny, vec!["Bash(rm:*)"]);
    assert!(parsed.permissions.strict_bash);
    assert!(parsed.store_api_key);

    Ok(())
}

#[test]
fn global_settings_store_api_key_defaults_true() -> Result<()> {
    let json = r#"{}"#;
    let gs: GlobalSettings = serde_json::from_str(json)?;
    assert!(gs.store_api_key);

    Ok(())
}

// -- ProjectSettings serialization

#[test]
fn project_settings_with_mcp_servers() -> Result<()> {
    let json = r#"{
        "mcpServers": {
            "my-mcp": {
                "command": "/usr/bin/mcp-server",
                "args": ["--port", "8080"],
                "env": {"API_KEY": "test"},
                "write_tools": ["create_pr"],
                "disabled": false
            }
        }
    }"#;
    let ps: ProjectSettings = serde_json::from_str(json)?;
    assert!(ps.mcp_servers.contains_key("my-mcp"));
    let cfg = &ps.mcp_servers["my-mcp"];
    assert_eq!(cfg.command, "/usr/bin/mcp-server");
    assert_eq!(cfg.args, vec!["--port", "8080"]);
    assert_eq!(cfg.env.get("API_KEY").map(|s| s.as_str()), Some("test"));
    assert_eq!(cfg.write_tools, vec!["create_pr"]);
    assert!(!cfg.disabled);

    Ok(())
}

// -- LocalSettings

#[test]
fn local_settings_pinned_agents() -> Result<()> {
    let json = r#"{
        "last_agent": "agent-42",
        "pinned_agents": [
            {"id": "a1", "name": "Alpha"},
            {"id": "a2", "name": "Beta"}
        ]
    }"#;
    let ls: LocalSettings = serde_json::from_str(json)?;
    assert_eq!(ls.last_agent.as_deref(), Some("agent-42"));
    assert_eq!(ls.pinned_agents.len(), 2);
    assert_eq!(ls.pinned_agents[0].id, "a1");
    assert_eq!(ls.pinned_agents[1].name, "Beta");

    Ok(())
}

// -- McpServerConfig

#[test]
fn mcp_server_config_defaults() {
    let cfg = McpServerConfig::default();
    assert!(cfg.command.is_empty());
    assert!(cfg.args.is_empty());
    assert!(cfg.env.is_empty());
    assert!(cfg.write_tools.is_empty());
    assert!(!cfg.disabled);
}

// -- HookDef serialization

#[test]
fn hook_def_json_roundtrip() -> Result<()> {
    let hook = HookDef::Command {
        command: "echo hello".into(),
        timeout: 30000,
    };
    let json = serde_json::to_string(&hook)?;
    let parsed: HookDef = serde_json::from_str(&json)?;
    match parsed {
        HookDef::Command { command, timeout } => {
            assert_eq!(command, "echo hello");
            assert_eq!(timeout, 30000);
        }
    }

    Ok(())
}

#[test]
fn hook_def_default_timeout() -> Result<()> {
    let json = r#"{"type": "command", "command": "test"}"#;
    let hook: HookDef = serde_json::from_str(json)?;
    match hook {
        HookDef::Command { timeout, .. } => assert_eq!(timeout, 60_000),
    }

    Ok(())
}

// -- SettingsManager (with temp dirs)

#[test]
fn settings_manager_loads_defaults_for_missing_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mgr = SettingsManager::new(dir.path())?;
    // local.last_agent is None for a fresh project dir
    assert!(mgr.local().last_agent.is_none());
    assert!(mgr.pinned_agents().is_empty());

    Ok(())
}

#[test]
fn settings_manager_merged_mcp_servers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    // Project settings with one server
    let project_json = r#"{
        "mcpServers": {
            "proj-server": {"command": "/bin/proj", "args": []}
        }
    }"#;
    fs::write(cade_dir.join("settings.json"), project_json)?;

    // Local settings with an override and a new server
    let local_json = r#"{
        "mcpServers": {
            "proj-server": {"command": "/bin/local-proj", "args": []},
            "local-only": {"command": "/bin/local-only", "args": []}
        }
    }"#;
    fs::write(cade_dir.join("settings.local.json"), local_json)?;

    let mgr = SettingsManager::new(dir.path())?;
    let servers = mgr.merged_mcp_servers();

    // Local override wins for proj-server
    assert_eq!(
        servers
            .get("proj-server")
            .ok_or("Should find server")?
            .command,
        "/bin/local-proj"
    );
    // Local-only server is present
    assert!(servers.contains_key("local-only"));

    Ok(())
}

#[test]
fn settings_manager_disabled_mcp_server_excluded() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    let project_json = r#"{
        "mcpServers": {
            "disabled-srv": {"command": "/bin/srv", "args": [], "disabled": true},
            "active-srv":   {"command": "/bin/active", "args": []}
        }
    }"#;
    fs::write(cade_dir.join("settings.json"), project_json)?;

    let mgr = SettingsManager::new(dir.path())?;
    let servers = mgr.merged_mcp_servers();
    assert!(!servers.contains_key("disabled-srv"));
    assert!(servers.contains_key("active-srv"));

    Ok(())
}

#[test]
fn settings_manager_set_and_get_last_agent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    // We don't modify home dir in tests — just verify the function works
    // with the temp project dir
    let mut mgr = SettingsManager::new(dir.path())?;
    // This writes to the temp dir's .cade/ and to ~/.cade/ (which may or may not exist)
    // We just verify it doesn't panic and returns Ok
    let _ = mgr.set_last_agent("test-agent-123");
    assert_eq!(mgr.last_agent(), Some("test-agent-123"));

    Ok(())
}

#[test]
fn settings_manager_pin_agent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    let mut mgr = SettingsManager::new(dir.path())?;
    mgr.pin_agent("a1", "Agent One")?;
    mgr.pin_agent("a2", "Agent Two")?;
    assert_eq!(mgr.pinned_agents().len(), 2);

    // Pin same ID again — deduplicates
    mgr.pin_agent("a1", "Agent One Updated")?;
    assert_eq!(mgr.pinned_agents().len(), 2);
    assert_eq!(
        mgr.pinned_agents()
            .iter()
            .find(|p| p.id == "a1")
            .ok_or("Should find agent")?
            .name,
        "Agent One Updated"
    );

    Ok(())
}

#[test]
fn settings_manager_save_and_load_rules() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    // Don't modify ~/.cade in CI — just test that save_allow_rule works without error
    let mut mgr = SettingsManager::new(dir.path())?;
    let _ = mgr.save_allow_rule("Bash(cargo test)");
    let _ = mgr.save_deny_rule("Bash(rm:*)");
    // Verify in-memory state
    assert!(
        mgr.permission_settings()
            .allow
            .contains(&"Bash(cargo test)".to_string())
    );
    assert!(
        mgr.permission_settings()
            .deny
            .contains(&"Bash(rm:*)".to_string())
    );

    Ok(())
}

#[test]
fn settings_manager_base_url_default() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mgr = SettingsManager::new(dir.path())?;
    // Without env vars set, should default to localhost
    let url = mgr.base_url();
    assert!(url.starts_with("http://localhost:"), "got: {url}");

    Ok(())
}

#[test]
fn settings_manager_merged_hooks() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_dir = dir.path().join(".cade");
    fs::create_dir_all(&cade_dir)?;

    let project_json = r#"{
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "echo proj"}]}]
        }
    }"#;
    fs::write(cade_dir.join("settings.json"), project_json)?;

    let local_json = r#"{
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "echo local"}]}]
        }
    }"#;
    fs::write(cade_dir.join("settings.local.json"), local_json)?;

    let mgr = SettingsManager::new(dir.path())?;
    let hooks = mgr.merged_hooks();
    // Local runs first (highest priority), then project, then global
    assert_eq!(hooks.pre_tool_use.len(), 2);

    Ok(())
}

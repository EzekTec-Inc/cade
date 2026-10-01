//! The test executable doubles as a real stdio MCP child. No shell, Python, or
//! platform-specific executable path is needed. `cargo test --test lifecycle`.
use cade_core::{
    capabilities::mesh::{CapabilityExecutionContext, CapabilityMesh},
    settings::McpServerConfig,
};
use cade_mcp::{McpManager, McpStartResult};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    sync::{Arc, atomic::Ordering},
};

fn fixture() {
    let marker = std::env::args().nth(2).unwrap_or_default();
    if let Some(path) = std::env::var_os("CADE_MCP_SPAWN_FILE") {
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap(),
            "spawn"
        )
        .unwrap();
    }
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => {
                if std::env::var_os("CADE_MCP_FAIL_FILE")
                    .is_some_and(|p| std::path::Path::new(&p).exists())
                {
                    return;
                }
                json!({"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}})
            }
            "tools/list" => {
                let is_write = std::env::var_os("CADE_MCP_WRITE_FILE")
                    .is_some_and(|p| std::path::Path::new(&p).exists());
                json!({"tools": [{"name": "echo", "description": format!("{marker}-{}", std::process::id()), "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": !is_write}}]})
            }
            "tools/call" => {
                let args = &request["params"]["arguments"];
                if args["commit_then_drop"] == true {
                    let path = std::env::var_os("CADE_MCP_COMMIT_FILE").unwrap();
                    writeln!(
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .unwrap(),
                        "committed"
                    )
                    .unwrap();
                    return;
                }
                if args["crash_pid"].as_u64() == Some(u64::from(std::process::id())) {
                    if args["permanent"] == true {
                        std::fs::write(std::env::var_os("CADE_MCP_FAIL_FILE").unwrap(), "fail")
                            .unwrap();
                    }
                    if args["change_policy"] == true {
                        std::fs::write(std::env::var_os("CADE_MCP_WRITE_FILE").unwrap(), "write")
                            .unwrap();
                    }
                    return;
                }
                json!({"content": [{"type": "text", "text": json!({"pid": std::process::id(), "marker": marker, "env": std::env::var("CADE_MCP_MARKER").unwrap_or_default()}).to_string()}], "isError": false})
            }
            "ping" => json!({}),
            _ => continue,
        };
        println!("{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        std::io::stdout().flush().unwrap();
    }
}

fn config(marker: &str, singleton: bool) -> McpServerConfig {
    McpServerConfig {
        command: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec!["--mcp-fixture".into(), marker.into()],
        singleton: Some(singleton),
        ..Default::default()
    }
}

async fn start(key: &str, cfg: McpServerConfig) -> McpManager {
    let (manager, results) = McpManager::start(&HashMap::from([(key.into(), cfg)]), None).await;
    assert!(
        matches!(
            results.as_slice(),
            [McpStartResult::Ok { tool_count: 1, .. }]
        ),
        "{results:?}"
    );
    manager
}

async fn echo(manager: &McpManager, key: &str) -> Value {
    let (text, error, _) = manager
        .call_tool(&format!("{key}__echo"), &json!({}))
        .await
        .unwrap()
        .unwrap();
    assert!(!error);
    serde_json::from_str(&text).unwrap()
}

async fn singleton_reconnect() {
    let manager = start("singleton", config("v1", true)).await;
    let before = echo(&manager, "singleton").await;
    let result = manager
        .call_tool("singleton__echo", &json!({"crash_pid": before["pid"]}))
        .await
        .unwrap();
    assert!(result.is_ok(), "{result:?}");
    assert_ne!(echo(&manager, "singleton").await["pid"], before["pid"]);
    assert_eq!(manager.status().await[0].status, "ready");
}

async fn reload_configuration() {
    let manager = start("reload", config("v1", true)).await;
    let before = echo(&manager, "reload").await;
    let mut changed = config("v2", true);
    changed
        .env
        .insert("CADE_MCP_MARKER".into(), "new-env".into());
    changed.core_server = true;
    changed.write_tools = vec!["echo".into()];
    let configs = HashMap::from([("reload".into(), changed.clone())]);
    let summary = manager.reload(&configs, None).await;
    assert_eq!(summary.started, ["reload"]);
    let after = echo(&manager, "reload").await;
    assert_ne!(after["pid"], before["pid"]);
    assert_eq!(after["marker"], "v2");
    assert_eq!(after["env"], "new-env");
    assert!(manager.is_write_tool("reload__echo").await);
    assert!(
        manager
            .active_catalog(&CapabilityExecutionContext::new("test"))
            .await[0]
            .tags
            .iter()
            .any(|tag| tag == "core_mcp")
    );
    assert_eq!(manager.reload(&configs, None).await.kept, ["reload"]);
    assert_eq!(echo(&manager, "reload").await["pid"], after["pid"]);
    changed.disabled = true;
    manager
        .reload(&HashMap::from([("reload".into(), changed.clone())]), None)
        .await;
    assert!(manager.all_tool_schemas().await.is_empty());
    assert!(
        manager
            .call_tool("reload__echo", &json!({}))
            .await
            .is_none()
    );
    changed.disabled = false;
    assert_eq!(
        manager
            .reload(&HashMap::from([("reload".into(), changed)]), None)
            .await
            .started,
        ["reload"]
    );
    assert_eq!(echo(&manager, "reload").await["marker"], "v2");
}

async fn reconnect_schema_change() {
    let manager = start("schema", config("v1", false)).await;
    let observer = manager.subscribe_catalog_changes();
    let before = manager.all_tool_schemas().await;
    let pid = echo(&manager, "schema").await["pid"].clone();
    assert!(
        manager
            .call_tool("schema__echo", &json!({"crash_pid": pid}))
            .await
            .unwrap()
            .is_ok()
    );
    let after = manager.all_tool_schemas().await;
    assert_ne!(before[0]["description"], after[0]["description"]);
    assert!(manager.schemas_dirty.load(Ordering::SeqCst));
    manager.schemas_dirty.store(false, Ordering::SeqCst);
    assert!(
        observer.has_changed().unwrap(),
        "a CLI observer cannot consume the server mirror's change"
    );
}

async fn unavailable_withdrawal_and_recovery() {
    let fail_file = std::env::temp_dir().join(format!("cade-mcp-failure-{}", std::process::id()));
    let _ = std::fs::remove_file(&fail_file);
    let mut cfg = config("v1", true);
    cfg.env.insert(
        "CADE_MCP_FAIL_FILE".into(),
        fail_file.to_string_lossy().into_owned(),
    );
    let manager = start("failed", cfg.clone()).await;
    let pid = echo(&manager, "failed").await["pid"].clone();
    let result = manager
        .call_tool(
            "failed__echo",
            &json!({"crash_pid": pid, "permanent": true}),
        )
        .await
        .unwrap();
    std::fs::remove_file(&fail_file).unwrap();
    assert!(result.is_err());
    assert!(
        manager
            .active_catalog(&CapabilityExecutionContext::new("test"))
            .await
            .is_empty()
    );
    assert!(manager.schemas_dirty.load(Ordering::SeqCst));
    assert_ne!(manager.status().await[0].status, "ready");
    assert_eq!(
        manager
            .reload(&HashMap::from([("failed".into(), cfg)]), None)
            .await
            .started,
        ["failed"]
    );
    assert_eq!(echo(&manager, "failed").await["marker"], "v1");
}

async fn reload_during_failure() {
    let manager = Arc::new(start("concurrent", config("v1", true)).await);
    let pid = echo(&manager, "concurrent").await["pid"].clone();
    let caller = manager.clone();
    let call = tokio::spawn(async move {
        caller
            .call_tool("concurrent__echo", &json!({"crash_pid": pid}))
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    manager.reload(&HashMap::new(), None).await;
    let _ = call
        .await
        .expect("reload must not invalidate a call's vector index");
    assert!(manager.all_tool_schemas().await.is_empty());
}

async fn reconnect_requires_reauthorization_for_changed_mutability() {
    let policy_file = std::env::temp_dir().join(format!("cade-mcp-policy-{}", std::process::id()));
    let _ = std::fs::remove_file(&policy_file);
    let mut cfg = config("v1", false);
    cfg.env.insert(
        "CADE_MCP_WRITE_FILE".into(),
        policy_file.to_string_lossy().into_owned(),
    );
    let manager = start("policy", cfg).await;
    assert!(!manager.is_write_tool("policy__echo").await);
    let pid = echo(&manager, "policy").await["pid"].clone();
    let result = manager
        .call_tool(
            "policy__echo",
            &json!({"crash_pid": pid, "change_policy": true}),
        )
        .await
        .unwrap();
    std::fs::remove_file(policy_file).unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("permissions changed")
    );
    assert!(manager.is_write_tool("policy__echo").await);
    assert_eq!(echo(&manager, "policy").await["marker"], "v1");
}

async fn wait_for_status(manager: &McpManager, expected: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if manager
                .status()
                .await
                .iter()
                .any(|status| status.status == expected)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("manager never reached {expected}"));
}

async fn cancelled_caller_does_not_cancel_recovery() {
    let manager = Arc::new(start("cancel", config("v1", true)).await);
    let pid = echo(&manager, "cancel").await["pid"].clone();
    let caller = manager.clone();
    let call = tokio::spawn(async move {
        caller
            .call_tool("cancel__echo", &json!({"crash_pid": pid}))
            .await
    });
    wait_for_status(&manager, "reconnecting").await;
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    wait_for_status(&manager, "ready").await;
    assert_eq!(echo(&manager, "cancel").await["marker"], "v1");
}

async fn dropping_manager_cancels_owned_recovery() {
    let path = std::env::temp_dir().join(format!("cade-mcp-spawns-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut cfg = config("v1", true);
    cfg.env.insert(
        "CADE_MCP_SPAWN_FILE".into(),
        path.to_string_lossy().into_owned(),
    );
    let manager = Arc::new(start("drop", cfg).await);
    let pid = echo(&manager, "drop").await["pid"].clone();
    let caller = manager.clone();
    let call = tokio::spawn(async move {
        caller
            .call_tool("drop__echo", &json!({"crash_pid": pid}))
            .await
    });
    wait_for_status(&manager, "reconnecting").await;
    call.abort();
    let _ = call.await;
    drop(manager);
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    std::fs::remove_file(path).unwrap();
}

async fn uncertain_write_is_never_replayed() {
    let path = std::env::temp_dir().join(format!("cade-mcp-commits-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut cfg = config("v1", true);
    cfg.write_tools = vec!["echo".into()];
    cfg.env.insert(
        "CADE_MCP_COMMIT_FILE".into(),
        path.to_string_lossy().into_owned(),
    );
    let manager = start("commit", cfg).await;
    let result = manager
        .call_tool("commit__echo", &json!({"commit_then_drop": true}))
        .await
        .unwrap();
    let commits = std::fs::read_to_string(&path).unwrap().lines().count();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        commits, 1,
        "a dropped response must not repeat an already committed write"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outcome is uncertain")
    );
    wait_for_status(&manager, "ready").await;
    assert_eq!(echo(&manager, "commit").await["marker"], "v1");
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("--mcp-fixture") {
        fixture();
        return;
    }
    let filter = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .unwrap_or_default();
    let mut failed = Vec::new();
    for name in [
        "singleton_reconnect",
        "reload_configuration",
        "reconnect_schema_change",
        "unavailable_withdrawal_and_recovery",
        "reload_during_failure",
        "reconnect_requires_reauthorization_for_changed_mutability",
        "cancelled_caller_does_not_cancel_recovery",
        "dropping_manager_cancels_owned_recovery",
        "uncertain_write_is_never_replayed",
    ] {
        if !name.contains(&filter) {
            continue;
        }
        let result = tokio::spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                match name {
                    "singleton_reconnect" => singleton_reconnect().await,
                    "reload_configuration" => reload_configuration().await,
                    "reconnect_schema_change" => reconnect_schema_change().await,
                    "unavailable_withdrawal_and_recovery" => {
                        unavailable_withdrawal_and_recovery().await
                    }
                    "reconnect_requires_reauthorization_for_changed_mutability" => {
                        reconnect_requires_reauthorization_for_changed_mutability().await
                    }
                    "cancelled_caller_does_not_cancel_recovery" => {
                        cancelled_caller_does_not_cancel_recovery().await
                    }
                    "dropping_manager_cancels_owned_recovery" => {
                        dropping_manager_cancels_owned_recovery().await
                    }
                    "uncertain_write_is_never_replayed" => {
                        uncertain_write_is_never_replayed().await
                    }
                    _ => reload_during_failure().await,
                }
            })
            .await
            .expect("bounded lifecycle operation");
        })
        .await;
        println!("{name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
        if result.is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "failed lifecycle tests: {failed:?}");
}

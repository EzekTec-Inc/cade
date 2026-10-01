//! Real ToolPipeline + self-hosted stdio MCP fixture. The first child exits
//! while idle, without receiving any tools/call request.
use cade_agent::{
    agent::HttpTransport,
    mcp::McpManager,
    tools::{AutoApprovalDelegate, ToolPipeline, ToolRuntime},
};
use cade_core::{
    hooks::HookEngine,
    permissions::{PermissionManager, PermissionMode},
    settings::{HooksConfig, McpServerConfig},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn append(path: &Path, text: &str) {
    writeln!(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap(),
        "{text}"
    )
    .unwrap();
}

fn fixture(root: PathBuf) {
    let first = !root.join("spawns").exists();
    append(&root.join("spawns"), "spawn");
    if first {
        let root = root.clone();
        std::thread::spawn(move || {
            while !root.join("exit").exists() {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::process::exit(0);
        });
    }
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => {
                json!({"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "idle-fixture", "version": "1"}})
            }
            "tools/list" => {
                json!({"tools": [{"name": "echo", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": !root.join("write").exists()}}]})
            }
            "tools/call" => {
                append(&root.join("calls"), "called");
                json!({"content": [{"type": "text", "text": "executed"}], "isError": false})
            }
            "ping" => json!({}),
            _ => continue,
        };
        println!("{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        std::io::stdout().flush().unwrap();
    }
}

async fn wait_status(manager: &McpManager, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if manager
                .status()
                .await
                .iter()
                .any(|server| server.status == expected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("manager never reached {expected}"));
}

async fn scenario(becomes_write: bool, cancel_binding: bool) {
    let root = tempfile::tempdir().unwrap();
    let config = McpServerConfig {
        command: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec![
            "--fixture".into(),
            root.path().to_string_lossy().into_owned(),
        ],
        singleton: Some(true),
        ..Default::default()
    };
    let (manager, _) = McpManager::start(&HashMap::from([("idle".into(), config)]), None).await;
    let manager = Arc::new(manager);
    let initial = manager.tool_binding("idle__echo").await.unwrap();
    assert!(!initial.1);
    let pipeline = Arc::new(ToolPipeline::new(
        Arc::new(ToolRuntime::new(
            Arc::new(HttpTransport::new("http://127.0.0.1:0".into(), "unused".into()).unwrap()),
            manager.clone(),
            "agent".into(),
            root.path().into(),
        )),
        PermissionManager::new(PermissionMode::Plan),
        Arc::new(HookEngine::new(
            HooksConfig::default(),
            root.path().into(),
            "session".into(),
        )),
        Arc::new(AutoApprovalDelegate),
    ));
    if becomes_write {
        std::fs::write(root.path().join("write"), "write").unwrap();
    }
    std::fs::write(root.path().join("exit"), "exit").unwrap();
    wait_status(&manager, "disconnected").await;
    assert!(
        manager.all_tool_schemas().await.is_empty(),
        "dead schemas must be withdrawn"
    );
    assert!(
        !root.path().join("calls").exists(),
        "no invocation caused the disconnect"
    );

    if cancel_binding {
        let caller = {
            let pipeline = pipeline.clone();
            tokio::spawn(async move {
                pipeline
                    .execute("cancelled", "idle__echo", &json!({}))
                    .await
            })
        };
        wait_status(&manager, "reconnecting").await;
        caller.abort();
        let _ = caller.await;
        wait_status(&manager, "ready").await;
        assert!(
            !root.path().join("calls").exists(),
            "recovery must not dispatch the cancelled intent"
        );
    }
    let result = pipeline
        .execute("fresh", "idle__echo", &json!({}))
        .await
        .unwrap();
    assert_eq!(result.is_error, becomes_write, "{}", result.output);
    assert_eq!(
        manager.status().await[0].status,
        "ready",
        "binding must recover before authorization"
    );
    let fresh = manager.tool_binding("idle__echo").await.unwrap();
    assert_ne!(initial.0, fresh.0);
    assert_eq!(fresh.1, becomes_write);
    assert_eq!(
        std::fs::read_to_string(root.path().join("spawns"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    if becomes_write {
        assert!(result.output.contains("Plan Mode"), "{}", result.output);
        assert!(
            !root.path().join("calls").exists(),
            "new mutability must be authorized before any call"
        );
    } else {
        assert_eq!(result.output, "executed");
        assert_eq!(
            std::fs::read_to_string(root.path().join("calls"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--fixture") {
        fixture(std::env::args().nth(2).unwrap().into());
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let filter = std::env::args().nth(1).unwrap_or_default();
    let mut failed = Vec::new();
    for (name, write, cancel) in [
        ("idle_read_recovers_through_pipeline", false, false),
        ("idle_recovery_authorizes_fresh_mutability", true, false),
        ("cancelled_binding_does_not_cancel_recovery", false, true),
    ] {
        if !name.contains(&filter) {
            continue;
        }
        let result = runtime.block_on(async {
            tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(20), scenario(write, cancel))
                    .await
                    .unwrap();
            })
            .await
        });
        println!("{name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
        if result.is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "failed idle recovery tests: {failed:?}");
}

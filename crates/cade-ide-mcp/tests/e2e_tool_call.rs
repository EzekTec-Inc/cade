//! End-to-end integration test for `cade-ide-mcp`.
//!
//! Spawns an [`IdeMcpServer`] on one end of a `tokio::io::duplex` pair
//! and a minimal rmcp client on the other, then asserts that calling
//! `get_active_file` returns `path: null` on a fresh `EditorState`.
//! This exercises the full rmcp plumbing — `initialize`, `tools/list`,
//! `tools/call` — without any process boundary or real stdio, so it's
//! hermetic and fast.

use cade_ide_mcp::{EditorState, IdeMcpServer};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;

#[tokio::test]
async fn get_active_file_on_empty_state_returns_null_path() {
    let server = IdeMcpServer::with_null_channel(EditorState::new());

    let (client_io, server_io) = tokio::io::duplex(4096);

    let server_handle = tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("serve server");
        running.waiting().await.expect("server wait");
    });

    let client = ().serve(client_io).await.expect("serve client");

    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new("get_active_file"))
        .await
        .expect("call_tool");

    let text = result
        .content
        .iter()
        .flat_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<String>();

    assert!(
        text.contains("\"path\":null") || text.contains("\"path\": null"),
        "expected path:null in tool result, got: {text}"
    );

    client.cancel().await.ok();
    server_handle.abort();
}

#[tokio::test]
async fn get_active_file_returns_path_pushed_by_adapter() {
    // Shared state — adapter and server hold clones of the same Arc.
    let state = EditorState::new();
    let server = IdeMcpServer::with_null_channel(state.clone());

    // Adapter side: push the active file *after* server is constructed,
    // the way a real editor extension would on "did change active editor".
    state.set_active_file(Some("/tmp/foo.rs".into())).await;

    let (client_io, server_io) = tokio::io::duplex(4096);
    let server_handle = tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("serve server");
        running.waiting().await.expect("server wait");
    });
    let client = ().serve(client_io).await.expect("serve client");

    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new("get_active_file"))
        .await
        .expect("call_tool");

    let text = result
        .content
        .iter()
        .flat_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<String>();

    assert!(
        text.contains("\"/tmp/foo.rs\""),
        "expected /tmp/foo.rs in tool result, got: {text}"
    );

    client.cancel().await.ok();
    server_handle.abort();
}

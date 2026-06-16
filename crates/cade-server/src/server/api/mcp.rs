//! `GET /v1/mcp` — list all MCP servers and their exposed tools.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::server::state::AppState;

/// `GET /v1/mcp`
///
/// Returns every MCP server currently loaded by the server, with its connection
/// command, tool list, and enabled/disabled status.
///
/// ```json
/// {
///   "servers": [
///     {
///       "key": "desktop-commander",
///       "command": "npx @desktop-commander/mcp-server",
///       "tools": ["bash", "read_file", "write_file", ...],
///       "disabled": false
///     }
///   ]
/// }
/// ```
pub async fn list_mcp_servers(State(state): State<AppState>) -> Json<Value> {
    let servers = state.mcp.status().await;
    Json(json!({ "servers": servers }))
}

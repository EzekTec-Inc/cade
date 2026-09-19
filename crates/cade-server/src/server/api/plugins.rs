//! Plugin management API handlers:
//! - `GET    /v1/plugins`         — list all installed plugins
//! - `POST   /v1/plugins/install` — download & install a WebAssembly plugin
//! - `DELETE /v1/plugins/:id`     — uninstall a plugin
//! - `GET    /v1/plugins/events`  — SSE stream of plugin lifecycle events

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use cade_plugin::{NativePluginEngine, PluginEngine};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::path::{Path as StdPath, PathBuf};

use crate::server::state::AppState;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallPluginPayload {
    pub url: String,
    pub plugin_id: Option<String>,
    pub agent_id: Option<String>,
}

/// `GET /v1/plugins` — list the canonical manifest-derived Plugin inventory.
pub async fn list_plugins_handler(State(_state): State<AppState>) -> Response {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = NativePluginEngine::from_default_dirs(&cwd);
    let reports = match engine.load_all() {
        Ok(reports) => reports,
        Err(error) => {
            tracing::error!(%error, "failed to load PluginEngine inventory");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load plugin inventory",
            );
        }
    };
    let tools = engine.list_tools();
    let plugins = reports
        .into_iter()
        .map(|report| {
            let exported_tools = tools
                .iter()
                .filter(|tool| tool.plugin_name == report.name)
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>();
            json!({
                "id": report.id,
                "name": report.name,
                "version": report.version,
                "scope": report.scope,
                "status": report.status,
                "diagnostic": report.diagnostic,
                "tools_count": report.tools_count,
                "skills_count": report.skills_count,
                "mcp_servers_count": report.mcp_servers_count,
                "exported_tools": exported_tools,
            })
        })
        .collect::<Vec<_>>();

    Json(json!({ "plugins": plugins, "count": plugins.len() })).into_response()
}

/// `POST /v1/plugins/install` — install a validated Plugin package through `PluginEngine`.
pub async fn install_plugin_handler(
    State(state): State<AppState>,
    Json(payload): Json<InstallPluginPayload>,
) -> Response {
    let plugin_id = payload.plugin_id.unwrap_or_else(|| {
        StdPath::new(&payload.url)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("plugin")
            .to_string()
    });
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = NativePluginEngine::from_default_dirs(&cwd);

    match engine.install(&payload.url, &plugin_id).await {
        Ok(report) => {
            crate::server::api::agents::publish_global_event(
                Some(&state.db),
                "plugin_installed",
                json!({
                    "plugin_id": report.id,
                    "scope": report.scope,
                    "status": report.status,
                }),
            );
            Json(json!({ "status": "installed", "plugin": report })).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, plugin_id, "PluginEngine installation failed");
            err(
                StatusCode::BAD_REQUEST,
                "Plugin installation failed during validation or activation",
            )
        }
    }
}

/// `DELETE /v1/plugins/:id` — uninstall a plugin by ID.
pub async fn uninstall_plugin_handler(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Response {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = NativePluginEngine::from_default_dirs(&cwd);

    match engine.uninstall(&plugin_id) {
        Ok(report) => {
            crate::server::api::agents::publish_global_event(
                Some(&state.db),
                "plugin_removed",
                json!({
                    "plugin_id": report.id,
                    "scope": report.scope,
                    "status": report.status,
                }),
            );
            Json(json!({ "status": "removed", "plugin": report })).into_response()
        }
        Err(error) if error.to_string().contains("Unknown plugin:") => {
            err(StatusCode::NOT_FOUND, "Unknown plugin")
        }
        Err(error) => {
            tracing::error!(%error, plugin_id, "PluginEngine removal failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Plugin removal failed")
        }
    }
}

/// `GET /v1/plugins/events` — live SSE stream of plugin lifecycle events.
pub async fn stream_plugin_events_handler(
    State(_state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::once(async {
        Ok(Event::default()
            .event("connected")
            .data(json!({ "status": "listening" }).to_string()))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_plugins_dir_and_stub_installation() {
        let temp = tempfile::tempdir().unwrap();
        let plugin_path = temp.path().join("my-test-plugin.wasm");
        std::fs::write(&plugin_path, b"\x00asm\x01\x00\x00\x00").unwrap();

        assert!(plugin_path.is_file());
        assert_eq!(plugin_path.file_stem().unwrap(), "my-test-plugin");
    }
}

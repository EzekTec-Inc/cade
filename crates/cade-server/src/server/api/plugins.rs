//! Plugin management API handlers:
//! - `GET    /v1/plugins`         — list all installed plugins
//! - `POST   /v1/plugins/install` — validate and activate a native plugin package
//! - `DELETE /v1/plugins/:id`     — uninstall a plugin
//! - `GET    /v1/plugins/events`  — SSE stream of plugin lifecycle events

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use cade_plugin::{NativePluginEngine, PluginEngine};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::path::Path as StdPath;

use crate::server::state::AppState;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallPluginPayload {
    pub url: String,
    pub plugin_id: Option<String>,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub registry_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchPluginsQuery {
    pub query: Option<String>,
    pub registry_url: Option<String>,
}

/// `GET /v1/plugins/search` — search the remote marketplace catalog for plugins.
pub async fn search_plugins_handler(
    State(_state): State<AppState>,
    Query(params): Query<SearchPluginsQuery>,
) -> Response {
    let registry_url = params.registry_url.unwrap_or_else(|| {
        std::env::var("CADE_REGISTRY_URL")
            .unwrap_or_else(|_| "https://registry.cade.dev".to_string())
    });
    let query = params.query.unwrap_or_default();
    let engine = default_engine();
    match engine.search_marketplace(&registry_url, &query).await {
        Ok(plugins) => Json(json!({
            "plugins": plugins,
            "count": plugins.len(),
            "query": query,
            "registry_url": registry_url,
        }))
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to search marketplace catalog");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Marketplace search failed: {error}"),
            )
        }
    }
}

/// `GET /v1/plugins` — list the canonical manifest-derived Plugin inventory.
pub async fn list_plugins_handler(State(_state): State<AppState>) -> Response {
    let engine = default_engine();
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
                .filter(|tool| {
                    tool.plugin_root.file_name().and_then(|name| name.to_str())
                        == Some(report.id.as_str())
                })
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>();
            json!({
                "id": report.id,
                "name": report.name,
                "version": report.version,
                "scope": report.scope,
                "status": report.status,
                "diagnostic": report.diagnostic,
                "tools_count": exported_tools.len(),
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
        let filename = payload
            .url
            .split(['?', '#'])
            .next()
            .unwrap_or(&payload.url)
            .rsplit('/')
            .next()
            .unwrap_or("plugin");
        let basename = filename
            .strip_suffix(".tar.gz")
            .or_else(|| filename.strip_suffix(".tgz"))
            .unwrap_or(filename);
        StdPath::new(basename)
            .file_name()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("plugin")
            .to_string()
    });
    let engine = default_engine();

    let install_res = if payload.url.starts_with("http://")
        || payload.url.starts_with("https://")
        || payload.url.starts_with("file://")
    {
        engine
            .install_with_checksum(&payload.url, &plugin_id, payload.sha256.as_deref())
            .await
    } else {
        let registry_url = payload.registry_url.unwrap_or_else(|| {
            std::env::var("CADE_REGISTRY_URL")
                .unwrap_or_else(|_| "https://registry.cade.dev".to_string())
        });
        engine
            .install_from_marketplace(&registry_url, &payload.url)
            .await
    };

    match install_res {
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
            err(StatusCode::BAD_REQUEST, &error.to_string())
        }
    }
}

/// `DELETE /v1/plugins/:id` — uninstall a plugin by ID.
pub async fn uninstall_plugin_handler(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Response {
    let engine = default_engine();

    if let Err(error) = cade_plugin::marketplace::validate_plugin_id(&plugin_id) {
        return err(StatusCode::BAD_REQUEST, &error.to_string());
    }

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
    let receiver = crate::server::api::agents::GLOBAL_EVENTS_TX.subscribe();
    let live =
        tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(|result| async move {
            let event = result.ok()?;
            let kind = event["event_type"].as_str()?;
            if !matches!(kind, "plugin_installed" | "plugin_removed") {
                return None;
            }
            Some(Ok(Event::default().event(kind).data(event.to_string())))
        });
    let connected = futures::stream::once(async {
        Ok(Event::default()
            .event("connected")
            .data("{\"status\":\"listening\"}"))
    });
    let stream = connected.chain(live);

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn default_engine() -> NativePluginEngine {
    let cwd = crate::server::api::run::runtime::execution_workspace();
    NativePluginEngine::from_default_dirs(&cwd)
}

#[cfg(test)]
mod tests {
    use cade_plugin::{NativePluginEngine, PluginEngine};

    #[tokio::test]
    async fn test_native_plugin_engine_inventory_and_tools() {
        let temp = tempfile::tempdir().unwrap();
        let plugin_dir = temp
            .path()
            .join(".cade")
            .join("plugins")
            .join("demo-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        // Create tool schema in tools/
        let tools_dir = plugin_dir.join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();
        let schema_path = tools_dir.join("demo_tool.json");
        std::fs::write(
            &schema_path,
            serde_json::json!({
                "name": "demo_tool",
                "description": "A demo plugin tool",
                "parameters": {"type": "object"}
            })
            .to_string(),
        )
        .unwrap();

        // Create cade-plugin.json
        let manifest_path = plugin_dir.join("cade-plugin.json");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "name": "Demo Plugin",
                "version": "1.2.3",
                "tools": [{"schema": "tools/demo_tool.json"}]
            })
            .to_string(),
        )
        .unwrap();

        let install_dir = temp.path().join(".cade/plugins");
        let engine = NativePluginEngine::new(vec![install_dir.clone()], install_dir);
        let reports = engine.load_all().expect("load_all should succeed");

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, "demo-plugin");
        assert_eq!(reports[0].name, "Demo Plugin");
        assert_eq!(reports[0].version, "1.2.3");
        assert_eq!(reports[0].scope, "project");
        assert_eq!(reports[0].tools_count, 0);

        let tools = engine.list_tools();
        assert!(
            tools.is_empty(),
            "declarations without handlers are not ready capabilities"
        );
        assert!(
            engine
                .dispatch("demo_tool", &serde_json::json!({}))
                .await
                .is_err()
        );
    }
}

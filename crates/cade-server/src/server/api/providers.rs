use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::server::state::AppState;
use cade_store::sqlite::{self, ProviderRow};
use cade_ai::{LlmRouter, PRESET_PROVIDERS};

fn server_err(msg: String) -> (StatusCode, Json<Value>) {
    tracing::error!("500 providers: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"detail": msg})),
    )
}

fn bad_req(msg: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({"detail": msg})))
}

/// GET /v1/providers — list all configured providers
pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db_rows = sqlite::list_providers(&state.db).map_err(|e| server_err(e.to_string()))?;

    let live_names = state.llm_router.read().await.provider_names();

    let providers: Vec<Value> = db_rows
        .iter()
        .map(|r| {
            json!({
                "name":     r.name,
                "kind":     r.kind,
                "base_url": r.base_url,
                "enabled":  r.enabled,
                "live":     live_names.contains(&r.name),
                // Never expose the API key
            })
        })
        .collect();

    // Also include env-var providers not in the DB
    let mut all = providers;
    for name in &live_names {
        if !db_rows.iter().any(|r| &r.name == name) {
            all.push(json!({
                "name":    name,
                "kind":    name,  // env-var providers have kind == name
                "enabled": true,
                "live":    true,
                "source":  "env"
            }));
        }
    }

    Ok(Json(json!({ "providers": all })))
}

/// POST /v1/providers — add or update a provider
///
/// Body: { "name": str, "kind": str, "api_key": str?, "base_url": str? }
///
/// kind: "anthropic" | "openai" | "gemini" | "ollama" | "openai-compatible"
/// For openai-compatible, base_url is required.
/// Preset shortcut: kind="preset" + name="openrouter"|"groq"|"together"|...
pub async fn add_provider(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let name = body["name"]
        .as_str()
        .ok_or_else(|| bad_req("'name' is required"))?
        .trim()
        .to_string();
    let mut kind = body["kind"]
        .as_str()
        .unwrap_or("openai-compatible")
        .trim()
        .to_string();
    let api_key = body["api_key"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let base_url = body["base_url"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if name.is_empty() {
        return Err(bad_req("'name' cannot be empty"));
    }

    // Preset shortcut: if kind == "preset" or kind == name and it matches a PresetDef, auto-fill base_url
    let base_url = if kind == "openai-compatible" || kind == "preset" || kind == name {
        if let Some(preset_url) = PRESET_PROVIDERS
            .iter()
            .find(|p| p.name == name.as_str())
            .map(|p| p.chat_url.to_string())
        {
            kind = "openai-compatible".to_string();
            base_url.or(Some(preset_url))
        } else {
            base_url
        }
    } else {
        base_url
    };

    // Validate kind
    match kind.as_str() {
        "anthropic" | "openai" | "gemini" | "ollama" | "openai-compatible" => {}
        other => {
            return Err(bad_req(&format!(
                "Invalid kind '{}'. Valid: anthropic, openai, gemini, ollama, openai-compatible",
                other
            )));
        }
    }
    if kind == "openai-compatible" && base_url.is_none() {
        return Err(bad_req(
            "'base_url' is required for openai-compatible providers",
        ));
    }

    let row = ProviderRow {
        name: name.clone(),
        kind: kind.clone(),
        api_key,
        base_url,
        enabled: true,
    };

    // Build the live provider and add to router — store API key so live model listing works.
    let ai_config = state.config.to_ai_config();
    if let Some(provider) = LlmRouter::provider_from_row(
        &row.kind,
        row.api_key.clone(),
        row.base_url.clone(),
        &ai_config,
    ) {
        let key = row.api_key.clone().unwrap_or_default();
        state
            .llm_router
            .write()
            .await
            .add_provider_with_key(name.clone(), provider, key);
    } else {
        return Err(bad_req(
            "Could not construct provider — check api_key/base_url",
        ));
    }

    // Persist to DB
    sqlite::upsert_provider(&state.db, &row).map_err(|e| server_err(e.to_string()))?;

    tracing::info!("Provider added: {} ({})", name, kind);
    Ok(Json(
        json!({ "name": name, "kind": kind, "status": "connected" }),
    ))
}

/// DELETE /v1/providers/:name — remove a provider
pub async fn remove_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let removed_live = state.llm_router.write().await.remove_provider(&name);
    let removed_db =
        sqlite::delete_provider(&state.db, &name).map_err(|e| server_err(e.to_string()))?;

    if !removed_live && !removed_db {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("Provider '{}' not found", name)})),
        ));
    }

    tracing::info!("Provider removed: {}", name);
    Ok(Json(json!({ "name": name, "status": "disconnected" })))
}

/// GET /v1/providers/presets — list available OpenAI-compatible presets
pub async fn list_presets() -> Json<Value> {
    let presets: Vec<Value> = PRESET_PROVIDERS
        .iter()
        .map(|p| {
            json!({
                "name":     p.name,
                "kind":     "openai-compatible",
                "base_url": p.chat_url,
                "models_url": p.models_url,
                "env_vars": p.env_vars,
            })
        })
        .collect();
    Json(json!({ "presets": presets }))
}

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::server::state::AppState;

/// GET /v1/models
///
/// All provider model lists are now fetched live — Anthropic, OpenAI, Gemini, Ollama,
/// and all preset providers (Groq, OpenRouter, etc.). Static catalogue is used only
/// as a per-provider fallback if the live endpoint is unreachable.
///
/// Hot-syncs env vars before listing: any API keys added to the shell after server
/// startup are picked up here, so the model picker always reflects current env state.
///
/// Returns:
/// - `supported`:        [] — kept for backward compat; all models now in `dynamic`
/// - `dynamic`:          live models from every configured provider, sorted by provider
/// - `custom_providers`: live providers with no known model listing (manually /connect-ed)
pub async fn list_models(State(state): State<AppState>) -> Json<Value> {
    // Hot-sync: pick up API keys added to env after server start (write lock held briefly)
    {
        let mut router = state.llm_router.write().await;
        router.hot_sync_env_providers();
    }

    let router = state.llm_router.read().await.clone();
    let live_names = router.provider_names();

    // All models — fetched live concurrently, with per-provider catalogue fallback
    let dynamic = router.list_dynamic_models().await;
    let listed: std::collections::HashSet<_> =
        dynamic.iter().map(|model| model.provider.clone()).collect();
    let custom_providers: Vec<String> = live_names
        .into_iter()
        .filter(|name| !listed.contains(name))
        .collect();
    let metadata: std::collections::BTreeMap<_, _> = dynamic
        .iter()
        .map(|entry| {
            let bare = entry
                .id
                .split_once('/')
                .map(|(_, id)| id)
                .unwrap_or(&entry.id);
            let registry = router.models.read();
            (
                entry.id.clone(),
                json!({
                    "capabilities": registry.metadata(&entry.provider, bare),
                    "source": registry.metadata_source(&entry.provider, bare),
                    "limits_are_fallback": !registry.has_known_limits(&entry.provider, bare),
                }),
            )
        })
        .collect();

    Json(json!({
        "supported":        [],
        "dynamic":          dynamic,
        "custom_providers": custom_providers,
        "metadata": metadata,
    }))
}

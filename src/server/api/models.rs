use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

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
pub async fn list_models(
    State(state): State<AppState>,
) -> Json<Value> {
    // Hot-sync: pick up API keys added to env after server start (write lock held briefly)
    {
        let mut router = state.llm_router.write().await;
        router.hot_sync_env_providers();
    }

    let router     = state.llm_router.read().await;
    let live_names = router.provider_names();

    // All models — fetched live concurrently, with per-provider catalogue fallback
    let dynamic = router.list_dynamic_models().await;
    drop(router);

    // Providers with no known model listing (not in catalogue, preset, or ollama)
    const KNOWN: &[&str] = &["anthropic", "openai", "gemini", "google", "ollama"];
    let all_known: std::collections::HashSet<&str> = KNOWN.iter()
        .chain(crate::server::llm::PRESET_PROVIDERS.iter().map(|p| &p.name))
        .copied()
        .collect();
    let custom_providers: Vec<String> = live_names.into_iter()
        .filter(|n| !all_known.contains(n.as_str()))
        .collect();

    Json(json!({
        "supported":        [],
        "dynamic":          dynamic,
        "custom_providers": custom_providers,
    }))
}

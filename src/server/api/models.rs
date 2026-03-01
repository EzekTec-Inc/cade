use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::server::llm::{catalogue::ModelEntry, CATALOGUE};
use crate::server::state::AppState;

/// GET /v1/models
///
/// Returns:
/// - `supported`: static catalogue models filtered to live standard providers
///               (anthropic, openai, gemini — catalogue is authoritative for these)
/// - `dynamic`:   live model lists from all providers that support querying:
///               Ollama (/api/tags), OpenRouter, Groq, Together, Fireworks, DeepInfra, etc.
///               Empty for a provider if it's unreachable or its key is missing.
/// - `custom_providers`: non-standard live providers without a known model list
///                       (kept for backward compatibility with the model picker)
///
/// The CLI model picker groups `dynamic` entries by provider name automatically.
pub async fn list_models(
    State(state): State<AppState>,
) -> Json<Value> {
    // Standard cloud providers whose models come from the static catalogue
    const CATALOGUE_PROVIDERS: &[&str] = &["anthropic", "openai", "gemini", "google"];

    let router     = state.llm_router.read().await;
    let live_names = router.provider_names();

    // 1. Supported — catalogue entries for live catalogue-backed providers
    let supported: Vec<ModelEntry> = CATALOGUE.iter()
        .filter(|(p, ..)| live_names.contains(&p.to_string()))
        .map(|e| ModelEntry::from_catalogue(e))
        .collect();

    // 2. Dynamic — live model lists from Ollama + preset providers with a /models endpoint
    let dynamic: Vec<ModelEntry> = router.list_dynamic_models().await;
    drop(router);

    // 3. Custom providers — live, not in catalogue set, no known model listing
    //    (providers added via /connect that aren't Ollama or a preset)
    let known: std::collections::HashSet<&str> = CATALOGUE_PROVIDERS.iter()
        .chain(crate::server::llm::PRESET_PROVIDERS.iter().map(|p| &p.name))
        .chain(std::iter::once(&"ollama"))
        .copied()
        .collect();
    let custom_providers: Vec<String> = live_names.into_iter()
        .filter(|n| !known.contains(n.as_str()))
        .collect();

    Json(json!({
        "supported":        supported,
        "dynamic":          dynamic,
        "custom_providers": custom_providers,
    }))
}

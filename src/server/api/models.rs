use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::server::llm::{catalogue::ModelEntry, CATALOGUE};
use crate::server::llm::ollama::OllamaProvider;
use crate::server::state::AppState;

/// GET /v1/models
///
/// Returns:
/// - `supported`: static catalogue models filtered to configured providers
/// - `dynamic`:   Ollama models queried live from `/api/tags` (empty if unreachable)
/// - `custom_providers`: provider names that are live but not in the standard set
///
/// The client uses this to populate the /model picker.
pub async fn list_models(
    State(state): State<AppState>,
) -> Json<Value> {
    let router     = state.llm_router.read().await;
    let live_names = router.provider_names();
    let ollama_url = router.ollama_base_url.clone();
    drop(router);

    // Standard cloud providers
    const STANDARD: &[&str] = &["anthropic", "openai", "gemini", "google", "ollama"];

    // 1. Supported — catalogue entries for live standard providers
    let supported: Vec<ModelEntry> = CATALOGUE.iter()
        .filter(|(p, ..)| live_names.contains(&p.to_string()))
        .map(|e| ModelEntry::from_catalogue(e))
        .collect();

    // 2. Dynamic — Ollama models from /api/tags (only if Ollama is live)
    let dynamic: Vec<ModelEntry> = if live_names.contains(&"ollama".to_string()) {
        let ollama = OllamaProvider::new(ollama_url);
        let tags = ollama.list_models().await;
        tags.into_iter().map(|name| ModelEntry {
            provider:     "ollama".to_string(),
            id:           format!("ollama/{name}"),
            display_name: format!("{name}"),
            toolset:      "default".to_string(),
            dynamic:      true,
        }).collect()
    } else {
        vec![]
    };

    // 3. Custom providers — live but not in the standard set
    let custom_providers: Vec<String> = live_names.into_iter()
        .filter(|n| !STANDARD.contains(&n.as_str()))
        .collect();

    Json(json!({
        "supported": supported,
        "dynamic":   dynamic,
        "custom_providers": custom_providers,
    }))
}

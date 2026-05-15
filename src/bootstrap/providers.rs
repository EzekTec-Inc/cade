use cade_agent::agent::HttpTransport;

/// Forward API keys from the CLI's environment to cade-server.
///
/// cade-server is a separate process and may not share the same environment.
/// This bridges the gap so that `export ANTHROPIC_API_KEY=...` in the user's
/// terminal is automatically propagated to the server.
pub async fn push_env_providers_to_server(client: &HttpTransport) {
    // (name, kind, env_vars, base_url)
    let core: &[(&str, &str, &[&str], Option<&str>)] = &[
        (
            "anthropic",
            "anthropic",
            &["ANTHROPIC_API_KEY", "CLAUDE_API_KEY"],
            None,
        ),
        ("openai", "openai", &["OPENAI_API_KEY"], None),
        (
            "gemini",
            "gemini",
            &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
            None,
        ),
    ];
    for (name, kind, vars, base_url) in core {
        let key = vars
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()));
        if let Some(key) = key {
            let _ = client.add_provider(name, kind, Some(&key), *base_url).await;
        }
    }
    // Preset OpenAI-compatible providers (Groq, OpenRouter, Together, etc.)
    // We load this from the registry so it dynamically picks up default_providers.json
    let presets = cade_ai::provider_registry::ProviderRegistry::new().get_all_providers().to_vec();
    for preset in presets {
        let key = preset
            .env_vars
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()));
        if let Some(key) = key {
            let _ = client
                .add_provider(&preset.name, "openai-compatible", Some(&key), Some(&preset.chat_url))
                .await;
        }
    }
}

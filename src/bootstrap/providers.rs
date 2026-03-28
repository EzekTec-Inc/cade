use cade_agent::agent::CadeClient;

/// Forward API keys from the CLI's environment to cade-server.
///
/// cade-server is a separate process and may not share the same environment.
/// This bridges the gap so that `export ANTHROPIC_API_KEY=...` in the user's
/// terminal is automatically propagated to the server.
pub async fn push_env_providers_to_server(client: &CadeClient) {
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
    let presets: &[(&str, &[&str], &str)] = &[
        (
            "openrouter",
            &["OPENROUTER_API_KEY"],
            "https://openrouter.ai/api/v1/chat/completions",
        ),
        (
            "groq",
            &["GROQ_API_KEY"],
            "https://api.groq.com/openai/v1/chat/completions",
        ),
        (
            "together",
            &["TOGETHER_API_KEY", "TOGETHER_AI_API_KEY"],
            "https://api.together.xyz/v1/chat/completions",
        ),
        (
            "fireworks",
            &["FIREWORKS_API_KEY"],
            "https://api.fireworks.ai/inference/v1/chat/completions",
        ),
        (
            "deepinfra",
            &["DEEPINFRA_API_KEY"],
            "https://api.deepinfra.com/v1/openai/chat/completions",
        ),
    ];
    for (name, vars, base_url) in presets {
        let key = vars
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()));
        if let Some(key) = key {
            let _ = client
                .add_provider(name, "openai-compatible", Some(&key), Some(base_url))
                .await;
        }
    }
}

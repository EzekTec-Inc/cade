use cade_ai::AiConfig;
use std::net::SocketAddr;

/// Runtime configuration for cade-server, resolved from env vars.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub db_path: String,
    pub llm_provider: LlmProviderKind,
    pub default_model: String,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub deepseek_api_key: Option<String>,
    pub ollama_base_url: String,
    /// Auth token required for CLI requests (optional; empty = no auth)
    pub api_key: Option<String>,
    /// Optional explicitly allowed CORS origin for remote deployments
    pub allowed_origin: Option<String>,
    /// Optional maximum context budget in characters
    pub max_context_budget: Option<usize>,
    /// Optional maximum tokens per turn
    pub max_tokens_per_turn: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmProviderKind {
    Anthropic,
    OpenAI,
    Gemini,
    DeepSeek,
    Ollama,
    /// Arbitrary configured provider name (its adapter kind is defined in providers.json/DB).
    Registered(String),
}

impl std::str::FromStr for LlmProviderKind {
    type Err = crate::server::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let definitions = cade_ai::provider_registry::ProviderRegistry::configured();
        let lower = s.to_lowercase();
        let definition = definitions.get(s).or_else(|| definitions.get(&lower));
        let name = definition.map(|p| p.name.as_str()).unwrap_or(s);
        let kind = match name.to_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" | "openai-compatible" => Ok(Self::OpenAI),
            "gemini" => Ok(Self::Gemini),
            "deepseek" => Ok(Self::DeepSeek),
            "ollama" => Ok(Self::Ollama),
            other
                if !other.is_empty()
                    && !other.contains('/')
                    && !other.chars().any(|c| c.is_whitespace() || c.is_control()) =>
            {
                Ok(Self::Registered(name.into()))
            }
            other => Err(crate::server::Error::custom(format!(
                "Invalid LLM provider name '{other}'"
            ))),
        }?;
        if definition.is_some() && kind.to_string() != name {
            return Ok(Self::Registered(name.into()));
        }
        Ok(kind)
    }
}

impl std::fmt::Display for LlmProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic => write!(f, "anthropic"),
            Self::OpenAI => write!(f, "openai"),
            Self::Gemini => write!(f, "gemini"),
            Self::DeepSeek => write!(f, "deepseek"),
            Self::Ollama => write!(f, "ollama"),
            Self::Registered(name) => write!(f, "{name}"),
        }
    }
}

/// Configured routing ID (provider + upstream ID), used when no explicit model is set.
pub fn default_model_for(provider: &LlmProviderKind) -> String {
    cade_ai::provider_registry::ProviderRegistry::configured()
        .get(&provider.to_string())
        .and_then(|p| {
            p.default_model
                .as_ref()
                .map(|model| format!("{}/{model}", p.name))
        })
        .unwrap_or_default()
}

/// Auto-detect the best available provider by scanning env keys.
/// Priority: Anthropic > OpenAI > Gemini > Ollama (always available as fallback).
/// Returns (provider, bare_model_name).
pub fn detect_provider() -> (LlmProviderKind, String) {
    // User-explicit override takes highest priority
    if let Ok(p) = std::env::var("CADE_LLM_PROVIDER")
        && let Ok(kind) = p.parse::<LlmProviderKind>()
    {
        // Allow explicit model override too
        let model = std::env::var("CADE_DEFAULT_MODEL")
            .unwrap_or_else(|_| default_model_for(&kind).to_string());
        return (kind, model);
    }

    let registry = cade_ai::provider_registry::ProviderRegistry::configured();
    let kind = registry
        .detected_default()
        .and_then(|p| p.name.parse().ok())
        .unwrap_or(LlmProviderKind::Ollama);
    let model = std::env::var("CADE_DEFAULT_MODEL").unwrap_or_else(|_| default_model_for(&kind));
    tracing::info!("Detected provider: {kind} → model: {model}");
    (kind, model)
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], 8284)),
            db_path: ":memory:".to_string(),
            llm_provider: LlmProviderKind::Ollama,
            default_model: default_model_for(&LlmProviderKind::Ollama),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            deepseek_api_key: None,
            ollama_base_url: cade_ai::provider_registry::ProviderRegistry::configured()
                .get("ollama")
                .map(|p| p.endpoint())
                .unwrap_or_default(),
            api_key: None,
            allowed_origin: None,
            max_context_budget: None,
            max_tokens_per_turn: None,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> crate::server::Result<Self> {
        Self::from_env_with_port(None)
    }

    pub fn from_env_with_port(port_override: Option<u16>) -> crate::server::Result<Self> {
        let port: u16 = port_override
            .or_else(|| {
                std::env::var("CADE_SERVER_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
            })
            .unwrap_or(8284);
        let host = std::env::var("CADE_SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let addr = configured_address(&host, port)?;

        let home = dirs::home_dir()
            .map(|h| {
                h.join(".cade")
                    .join("cade.db")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| "cade.db".to_string());
        let db_path = std::env::var("CADE_DB_PATH").unwrap_or(home);

        let (llm_provider, default_model) = detect_provider();
        if default_model.trim().is_empty() {
            return Err(crate::server::Error::custom(format!(
                "Provider '{llm_provider}' has no configured default model; set CADE_DEFAULT_MODEL or default_model in providers.json"
            )));
        }

        let mut max_context_budget = std::env::var("CADE_MAX_CONTEXT_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok());
        let mut max_tokens_per_turn = std::env::var("CADE_MAX_TOKENS_PER_TURN")
            .ok()
            .and_then(|v| v.parse().ok());

        if let Some(home) = dirs::home_dir() {
            let settings_path = home.join(".cade").join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&settings_path)
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
            {
                if max_context_budget.is_none()
                    && let Some(budget) = json.get("max_context_budget").and_then(|v| v.as_u64())
                {
                    max_context_budget = Some(budget as usize);
                }
                if max_tokens_per_turn.is_none()
                    && let Some(tokens) = json.get("max_tokens_per_turn").and_then(|v| v.as_u64())
                {
                    max_tokens_per_turn = Some(tokens as usize);
                }
            }
        }

        Ok(Self {
            addr,
            db_path,
            default_model,
            llm_provider,
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("CLAUDE_API_KEY"))
                .ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            google_api_key: std::env::var("GOOGLE_API_KEY")
                .or_else(|_| std::env::var("GEMINI_API_KEY"))
                .ok(),
            deepseek_api_key: std::env::var("DEEPSEEK_API_KEY").ok(),
            ollama_base_url: std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| {
                cade_ai::provider_registry::ProviderRegistry::configured()
                    .get("ollama")
                    .map(|p| p.endpoint())
                    .unwrap_or_default()
            }),
            api_key: resolve_api_key(),
            allowed_origin: std::env::var("CADE_ALLOWED_ORIGIN").ok(),
            max_context_budget,
            max_tokens_per_turn,
        })
    }

    /// Convert to the provider-agnostic `AiConfig` used by `cade-ai`.
    pub fn to_ai_config(&self) -> AiConfig {
        AiConfig {
            anthropic_api_key: self.anthropic_api_key.clone(),
            openai_api_key: self.openai_api_key.clone(),
            google_api_key: self.google_api_key.clone(),
            deepseek_api_key: self.deepseek_api_key.clone(),
            ollama_base_url: self.ollama_base_url.clone(),
            llm_provider: self.llm_provider.to_string(),
        }
    }
}

/// Resolve the server's bearer-auth token.
///
/// Priority:
///   1. `CADE_API_KEY` env var (non-empty) — explicit user override
///   2. Persistent token at `~/.cade/api-token` — created on first launch
///
/// When the persistent token file cannot be created (e.g. no home directory,
/// unwritable filesystem), falls back to `None`; the auth middleware will
/// then reject every non-health request with 401.
fn resolve_api_key() -> Option<String> {
    if let Ok(k) = std::env::var("CADE_API_KEY") {
        let trimmed = k.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let path = crate::server::bootstrap::default_token_path()?;
    match crate::server::bootstrap::load_or_create_token(&path) {
        Ok(token) => Some(token),
        Err(e) => {
            tracing::error!("Failed to load/create API token at {}: {e}", path.display());
            None
        }
    }
}

fn configured_address(host: &str, port: u16) -> crate::server::Result<SocketAddr> {
    Ok(SocketAddr::new(host.parse::<std::net::IpAddr>()?, port))
}

#[cfg(test)]
mod address_tests {
    use super::configured_address;

    #[test]
    fn configured_listeners_accept_ipv4_and_ipv6() {
        assert_eq!(
            configured_address("0.0.0.0", 8284).unwrap().to_string(),
            "0.0.0.0:8284"
        );
        assert_eq!(
            configured_address("::1", 8284).unwrap().to_string(),
            "[::1]:8284"
        );
        assert!(configured_address("invalid-host", 8284).is_err());
    }
}

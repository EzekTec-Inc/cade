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
    pub ollama_base_url: String,
    /// Auth token required for CLI requests (optional; empty = no auth)
    pub api_key: Option<String>,
    /// Optional explicitly allowed CORS origin for remote deployments
    pub allowed_origin: Option<String>,
    /// Optional maximum context budget in characters
    pub max_context_budget: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmProviderKind {
    Anthropic,
    OpenAI,
    Gemini,
    Ollama,
}

impl std::str::FromStr for LlmProviderKind {
    type Err = crate::server::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "openai" | "openai-compatible" => Ok(Self::OpenAI),
            "gemini" | "google" => Ok(Self::Gemini),
            "ollama" | "local" => Ok(Self::Ollama),
            other => Err(crate::server::Error::custom(format!(
                "Unknown LLM provider '{other}'. Valid: anthropic, openai, gemini, ollama"
            ))),
        }
    }
}

impl std::fmt::Display for LlmProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic => write!(f, "anthropic"),
            Self::OpenAI => write!(f, "openai"),
            Self::Gemini => write!(f, "gemini"),
            Self::Ollama => write!(f, "ollama"),
        }
    }
}

/// Best-in-class model for each provider (used when no explicit model is set)
pub fn default_model_for(provider: &LlmProviderKind) -> &'static str {
    match provider {
        LlmProviderKind::Anthropic => "claude-opus-4-5",
        LlmProviderKind::OpenAI => "gpt-4o",
        LlmProviderKind::Gemini => "gemini-2.5-pro",
        LlmProviderKind::Ollama => "llama3.2", // most likely installed; user can override
    }
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

    // Scan for API keys in priority order
    let providers: &[(fn() -> bool, LlmProviderKind)] = &[
        (
            || {
                std::env::var("ANTHROPIC_API_KEY")
                    .map(|k| !k.is_empty())
                    .unwrap_or(false)
            },
            LlmProviderKind::Anthropic,
        ),
        (
            || {
                std::env::var("OPENAI_API_KEY")
                    .map(|k| !k.is_empty())
                    .unwrap_or(false)
            },
            LlmProviderKind::OpenAI,
        ),
        (
            || {
                std::env::var("GOOGLE_API_KEY")
                    .map(|k| !k.is_empty())
                    .unwrap_or(false)
            },
            LlmProviderKind::Gemini,
        ),
    ];

    for (check, kind) in providers {
        if check() {
            let model = std::env::var("CADE_DEFAULT_MODEL")
                .unwrap_or_else(|_| default_model_for(kind).to_string());
            tracing::info!("Auto-detected provider: {} → model: {}", kind, model);
            return (kind.clone(), model);
        }
    }

    // Ollama is always available as local fallback
    let model = std::env::var("CADE_DEFAULT_MODEL")
        .unwrap_or_else(|_| default_model_for(&LlmProviderKind::Ollama).to_string());
    tracing::info!("No API keys found — falling back to Ollama ({})", model);
    (LlmProviderKind::Ollama, model)
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
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;

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

        let mut max_context_budget = std::env::var("CADE_MAX_CONTEXT_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok());
        if max_context_budget.is_none()
            && let Some(home) = dirs::home_dir()
        {
            let settings_path = home.join(".cade").join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&settings_path)
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(budget) = json.get("max_context_budget").and_then(|v| v.as_u64())
            {
                max_context_budget = Some(budget as usize);
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
            ollama_base_url: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            api_key: resolve_api_key(),
            allowed_origin: std::env::var("CADE_ALLOWED_ORIGIN").ok(),
            max_context_budget,
        })
    }

    /// Convert to the provider-agnostic `AiConfig` used by `cade-ai`.
    pub fn to_ai_config(&self) -> AiConfig {
        AiConfig {
            anthropic_api_key: self.anthropic_api_key.clone(),
            openai_api_key: self.openai_api_key.clone(),
            google_api_key: self.google_api_key.clone(),
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
    if let Ok(k) = std::env::var("CADE_API_KEY")
        && !k.is_empty()
    {
        return Some(k);
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

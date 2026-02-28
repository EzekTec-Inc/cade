use std::net::SocketAddr;

/// Runtime configuration for cade-server, resolved from env vars.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub db_path: String,
    pub llm_provider: LlmProviderKind,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub ollama_base_url: String,
    /// Auth token required for CLI requests (optional; empty = no auth)
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmProviderKind {
    Anthropic,
    OpenAI,
    Gemini,
    Ollama,
}

impl std::str::FromStr for LlmProviderKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "openai" | "openai-compatible" => Ok(Self::OpenAI),
            "gemini" | "google" => Ok(Self::Gemini),
            "ollama" | "local" => Ok(Self::Ollama),
            other => Err(anyhow::anyhow!("Unknown LLM provider: '{other}'. Valid: anthropic, openai, gemini, ollama")),
        }
    }
}

impl std::fmt::Display for LlmProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic => write!(f, "anthropic"),
            Self::OpenAI    => write!(f, "openai"),
            Self::Gemini    => write!(f, "gemini"),
            Self::Ollama    => write!(f, "ollama"),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let port: u16 = std::env::var("CADE_SERVER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8284);
        let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;

        let home = dirs::home_dir()
            .map(|h| h.join(".cade").join("cade.db").to_string_lossy().to_string())
            .unwrap_or_else(|| "cade.db".to_string());
        let db_path = std::env::var("CADE_DB_PATH").unwrap_or(home);

        let llm_provider: LlmProviderKind = std::env::var("CADE_LLM_PROVIDER")
            .unwrap_or_else(|_| "anthropic".to_string())
            .parse()?;

        Ok(Self {
            addr,
            db_path,
            llm_provider,
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            google_api_key: std::env::var("GOOGLE_API_KEY").ok(),
            ollama_base_url: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            api_key: std::env::var("CADE_API_KEY").ok(),
        })
    }
}

/// Configuration for the intelligent tool selection reranker.
#[derive(Debug, Clone)]
pub struct RerankerConfig {
    /// Enable intelligent tool selection.
    pub enabled: bool,

    /// Number of top tools to keep after reranking.
    pub top_n: usize,

    /// Which backend to use.
    pub backend: RerankerBackend,

    /// Tool names that must never be pruned regardless of score.
    pub protected_tools: Vec<String>,
}

/// Reranker backend selection.
#[derive(Debug, Clone)]
pub enum RerankerBackend {
    /// Local ONNX cross-encoder (default — no API key required).
    #[cfg(feature = "local")]
    Local {
        /// Override the default model path.
        model_path: Option<std::path::PathBuf>,
    },

    /// Cohere Rerank API.
    Cohere { api_key: String },

    /// Voyage AI Rerank API.
    Voyage { api_key: String },

    /// Jina AI Rerank API.
    Jina { api_key: String },
}

// -- Defaults

impl Default for RerankerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            top_n: 15,
            backend: RerankerBackend::default(),
            protected_tools: default_protected_tools(),
        }
    }
}

impl Default for RerankerBackend {
    fn default() -> Self {
        #[cfg(feature = "local")]
        {
            Self::Local { model_path: None }
        }
        #[cfg(not(feature = "local"))]
        {
            // Without the `local` feature, there is no zero-config backend.
            // The user must provide a cloud API key.
            panic!("cade-reranker: no default backend — enable the `local` feature or configure a cloud provider");
        }
    }
}

/// Tools that are ALWAYS included regardless of reranking score.
///
/// These are the agent's lifeline for context recovery and core coding —
/// pruning them would silently break the agent.
pub fn default_protected_tools() -> Vec<String> {
    [
        // Memory / retrieval — agent's primary context recovery mechanism
        "search_memory",
        "conversation_search",
        "archival_memory_insert",
        "archival_memory_search",
        "update_memory",
        "update_memory_typed",
        "memory_apply_patch",
        // Core coding — almost always needed
        "bash",
        "read_file",
        "RunShellCommand",
        "ReadFileGemini",
        // User interaction
        "ask_user_question",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Build a [`RerankerConfig`] from environment variables.
///
/// | Variable | Default | Description |
/// |----------|---------|-------------|
/// | `CADE_RERANKER_ENABLED` | `false` | Enable intelligent tool selection |
/// | `CADE_RERANKER_TOP_N` | `15` | Number of top tools to keep |
/// | `CADE_RERANKER_BACKEND` | `local` | `local`, `cohere`, `voyage`, `jina` |
/// | `COHERE_API_KEY` | — | Cohere API key (for `cohere` backend) |
/// | `VOYAGE_API_KEY` | — | Voyage AI API key (for `voyage` backend) |
/// | `JINA_API_KEY` | — | Jina AI API key (for `jina` backend) |
pub fn config_from_env() -> RerankerConfig {
    let enabled = std::env::var("CADE_RERANKER_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let top_n = std::env::var("CADE_RERANKER_TOP_N")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15);

    let backend_name = std::env::var("CADE_RERANKER_BACKEND")
        .unwrap_or_else(|_| "local".to_string());

    let backend = match backend_name.to_lowercase().as_str() {
        "cohere" => {
            let key = std::env::var("COHERE_API_KEY").unwrap_or_default();
            RerankerBackend::Cohere { api_key: key }
        }
        "voyage" => {
            let key = std::env::var("VOYAGE_API_KEY").unwrap_or_default();
            RerankerBackend::Voyage { api_key: key }
        }
        "jina" => {
            let key = std::env::var("JINA_API_KEY").unwrap_or_default();
            RerankerBackend::Jina { api_key: key }
        }
        #[cfg(feature = "local")]
        _ => {
            let model_path = std::env::var("CADE_RERANKER_MODEL_PATH")
                .ok()
                .map(std::path::PathBuf::from);
            RerankerBackend::Local { model_path }
        }
        #[cfg(not(feature = "local"))]
        _ => {
            tracing::warn!(
                "CADE_RERANKER_BACKEND='{}' but `local` feature is disabled — falling back",
                backend_name
            );
            RerankerBackend::default()
        }
    };

    RerankerConfig {
        enabled,
        top_n,
        backend,
        protected_tools: default_protected_tools(),
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled() {
        let cfg = RerankerConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.top_n, 15);
    }

    #[test]
    fn protected_tools_include_essentials() {
        let tools = default_protected_tools();
        assert!(tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"search_memory".to_string()));
    }
}

// endregion: --- Tests

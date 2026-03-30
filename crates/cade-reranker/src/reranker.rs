use crate::config::{RerankerBackend, RerankerConfig};
use crate::{Error, Result};
use serde_json::Value;
use std::collections::HashSet;

/// A tool document prepared for reranking.
#[derive(Debug, Clone)]
pub struct ToolDocument {
    /// Original JSON schema (passed through to the LLM if selected).
    pub schema: Value,
    /// Tool name extracted from the schema.
    pub name: String,
    /// Human-readable text representation used for scoring.
    pub text: String,
}

/// Outcome of a rerank operation.
pub struct RerankResult {
    /// The filtered and reordered tool schemas.
    pub schemas: Vec<Value>,
    /// Number of tools before reranking.
    pub original_count: usize,
    /// Number of tools after reranking.
    pub selected_count: usize,
    /// Wall-clock time for the rerank operation.
    pub elapsed_ms: u64,
}

/// The top-level reranker.  Holds configuration and — when the `local`
/// feature is active — the lazily-initialised ONNX session.
pub struct ToolReranker {
    config: RerankerConfig,

    /// Lazy-initialised local model (behind `local` feature).
    #[cfg(feature = "local")]
    local: tokio::sync::OnceCell<crate::model::LocalModel>,
}

impl ToolReranker {
    /// Create a new reranker from the given config.
    /// The underlying model is NOT loaded until the first `rerank()` call.
    pub fn new(config: RerankerConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "local")]
            local: tokio::sync::OnceCell::new(),
        }
    }

    /// Whether the reranker is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Read-only access to the config.
    pub fn config(&self) -> &RerankerConfig {
        &self.config
    }

    // -- Tool → Document conversion

    /// Convert a tool JSON schema into a [`ToolDocument`] for scoring.
    ///
    /// The text representation follows the format that benchmarks best for
    /// cross-encoder reranking:
    ///
    /// ```text
    /// {name}
    /// {description}
    /// param1: param1_description, param2: param2_description, ...
    /// ```
    pub fn schema_to_document(schema: &Value) -> ToolDocument {
        let name = schema["name"].as_str().unwrap_or("").to_string();

        let mut parts: Vec<String> = Vec::new();
        parts.push(name.clone());

        if let Some(desc) = schema["description"].as_str() {
            parts.push(desc.to_string());
        }

        if let Some(props) = schema["parameters"]["properties"].as_object() {
            let param_parts: Vec<String> = props
                .iter()
                .map(|(k, v)| {
                    if let Some(d) = v["description"].as_str() {
                        format!("{k}: {d}")
                    } else {
                        k.to_string()
                    }
                })
                .collect();
            if !param_parts.is_empty() {
                parts.push(param_parts.join(", "));
            }
        }

        ToolDocument {
            schema: schema.clone(),
            name,
            text: parts.join("\n"),
        }
    }

    // -- Main rerank entry point

    /// Rerank the given tool schemas against a user prompt.
    ///
    /// Returns all tools if:
    /// - The reranker is disabled
    /// - The tool count is already ≤ top_n + protected count
    /// - An error occurs (graceful fallback)
    pub async fn rerank(
        &self,
        user_prompt: &str,
        tool_schemas: Vec<Value>,
    ) -> Vec<Value> {
        if !self.config.enabled {
            return tool_schemas;
        }

        let protected: HashSet<&str> = self
            .config
            .protected_tools
            .iter()
            .map(String::as_str)
            .collect();

        // Separate protected from rankable tools.
        let mut always_keep: Vec<Value> = Vec::new();
        let mut candidates: Vec<Value> = Vec::new();
        for schema in &tool_schemas {
            let name = schema["name"].as_str().unwrap_or("");
            if protected.contains(name) {
                always_keep.push(schema.clone());
            } else {
                candidates.push(schema.clone());
            }
        }

        // If the candidates plus protected are already within budget, skip.
        let budget = self.config.top_n.saturating_sub(always_keep.len());
        if candidates.len() <= budget {
            tracing::debug!(
                "[reranker] {} candidates ≤ budget {}, skipping",
                candidates.len(),
                budget
            );
            return tool_schemas;
        }

        // Convert to documents.
        let docs: Vec<ToolDocument> = candidates
            .iter()
            .map(|s| Self::schema_to_document(s))
            .collect();

        let original_count = tool_schemas.len();
        let start = std::time::Instant::now();

        // Dispatch to the configured backend.
        let ranked = match &self.config.backend {
            #[cfg(feature = "local")]
            RerankerBackend::Local { model_path } => {
                self.rerank_local(user_prompt, &docs, budget, model_path.as_deref())
                    .await
            }
            RerankerBackend::Cohere { api_key } => {
                rerank_cohere(api_key, user_prompt, &docs, budget).await
            }
            RerankerBackend::Voyage { api_key } => {
                rerank_voyage(api_key, user_prompt, &docs, budget).await
            }
            RerankerBackend::Jina { api_key } => {
                rerank_jina(api_key, user_prompt, &docs, budget).await
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        match ranked {
            Ok(selected_docs) => {
                let selected_count = always_keep.len() + selected_docs.len();
                tracing::info!(
                    "[reranker] {original_count} → {selected_count} tools in {elapsed_ms}ms"
                );

                let mut result = always_keep;
                result.extend(selected_docs.into_iter().map(|d| d.schema));
                result
            }
            Err(e) => {
                tracing::warn!("[reranker] failed ({e}), returning full tool set");
                tool_schemas
            }
        }
    }

    // -- Local ONNX backend

    #[cfg(feature = "local")]
    async fn rerank_local(
        &self,
        user_prompt: &str,
        docs: &[ToolDocument],
        top_n: usize,
        model_path: Option<&std::path::Path>,
    ) -> Result<Vec<ToolDocument>> {
        let model = self
            .local
            .get_or_try_init(|| async { crate::model::LocalModel::load(model_path).await })
            .await?;

        model.rerank(user_prompt, docs, top_n)
    }
}

// region:    --- Cloud backends

/// Cohere `/v2/rerank` API.
async fn rerank_cohere(
    api_key: &str,
    query: &str,
    docs: &[ToolDocument],
    top_n: usize,
) -> Result<Vec<ToolDocument>> {
    let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();
    let body = serde_json::json!({
        "model": "rerank-v3.5",
        "query": query,
        "documents": texts,
        "top_n": top_n,
    });

    let resp = reqwest::Client::new()
        .post("https://api.cohere.com/v2/rerank")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::custom(format!("Cohere rerank {status}: {text}")));
    }

    let json: Value = resp.json().await?;
    parse_index_results(&json["results"], docs, top_n)
}

/// Voyage AI `/v1/rerank` API.
async fn rerank_voyage(
    api_key: &str,
    query: &str,
    docs: &[ToolDocument],
    top_n: usize,
) -> Result<Vec<ToolDocument>> {
    let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();
    let body = serde_json::json!({
        "model": "rerank-2.5",
        "query": query,
        "documents": texts,
        "top_k": top_n,
    });

    let resp = reqwest::Client::new()
        .post("https://api.voyageai.com/v1/rerank")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::custom(format!("Voyage rerank {status}: {text}")));
    }

    let json: Value = resp.json().await?;
    parse_index_results(&json["results"], docs, top_n)
}

/// Jina AI `/v1/rerank` API.
async fn rerank_jina(
    api_key: &str,
    query: &str,
    docs: &[ToolDocument],
    top_n: usize,
) -> Result<Vec<ToolDocument>> {
    let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();
    let body = serde_json::json!({
        "model": "jina-reranker-v2-base-multilingual",
        "query": query,
        "documents": texts,
        "top_n": top_n,
    });

    let resp = reqwest::Client::new()
        .post("https://api.jina.ai/v1/rerank")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::custom(format!("Jina rerank {status}: {text}")));
    }

    let json: Value = resp.json().await?;
    parse_index_results(&json["results"], docs, top_n)
}

/// Parse the `results` array returned by Cohere / Voyage / Jina APIs.
///
/// Each element has `{ "index": N, "relevance_score": F }`.
fn parse_index_results(
    results: &Value,
    docs: &[ToolDocument],
    top_n: usize,
) -> Result<Vec<ToolDocument>> {
    let arr = results
        .as_array()
        .ok_or_else(|| Error::custom("expected results array from rerank API"))?;

    let mut selected: Vec<ToolDocument> = Vec::with_capacity(top_n);
    for item in arr.iter().take(top_n) {
        let idx = item["index"]
            .as_u64()
            .ok_or_else(|| Error::custom("missing index in rerank result"))? as usize;
        if idx < docs.len() {
            selected.push(docs[idx].clone());
        }
    }
    Ok(selected)
}

// endregion: --- Cloud backends

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_to_document_basic() {
        let schema = json!({
            "name": "bash",
            "description": "Execute a shell command.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout": { "type": "integer", "description": "Timeout in seconds" }
                }
            }
        });

        let doc = ToolReranker::schema_to_document(&schema);
        assert_eq!(doc.name, "bash");
        assert!(doc.text.contains("bash"));
        assert!(doc.text.contains("Execute a shell command."));
        assert!(doc.text.contains("command: The command to execute"));
        assert!(doc.text.contains("timeout: Timeout in seconds"));
    }

    #[test]
    fn schema_to_document_no_params() {
        let schema = json!({
            "name": "list_agents",
            "description": "List all active agents.",
            "parameters": { "type": "object", "properties": {} }
        });

        let doc = ToolReranker::schema_to_document(&schema);
        assert_eq!(doc.name, "list_agents");
        assert!(doc.text.contains("List all active agents."));
    }

    #[tokio::test]
    async fn disabled_reranker_passes_through() {
        let config = RerankerConfig {
            enabled: false,
            ..Default::default()
        };
        let reranker = ToolReranker::new(config);
        let tools = vec![json!({"name": "bash"}), json!({"name": "grep"})];
        let result = reranker.rerank("hello", tools.clone()).await;
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn within_budget_skips_reranking() {
        let config = RerankerConfig {
            enabled: true,
            top_n: 50, // way more than we'll pass in
            ..Default::default()
        };
        let reranker = ToolReranker::new(config);
        let tools = vec![json!({"name": "bash"}), json!({"name": "grep"})];
        let result = reranker.rerank("hello", tools.clone()).await;
        assert_eq!(result.len(), 2);
    }
}

// endregion: --- Tests

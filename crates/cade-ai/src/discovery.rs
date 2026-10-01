//! Protocol-based discovery against the registered endpoint, never provider-name dispatch.
use crate::{Error, ModelEntry, Result, SharedModelRegistry, runtime::ModelMetadata};
use serde_json::Value;

pub(crate) async fn default_models(provider: &str, key: &str) -> Vec<ModelEntry> {
    let definitions = crate::provider_registry::ProviderRegistry::configured();
    let Some(definition) = definitions.get(provider) else {
        return Vec::new();
    };
    if !definition.discovery {
        return Vec::new();
    }
    let endpoint = definition.endpoint();
    let url = if endpoint != definition.chat_url {
        models_endpoint(&definition.kind, &endpoint)
    } else {
        definition
            .models_url
            .clone()
            .or_else(|| models_endpoint(&definition.kind, &endpoint))
    };
    let Some(url) = url else {
        return Vec::new();
    };
    match discover_with_headers(
        &definition.kind,
        &url,
        key,
        &definition.name,
        &crate::runtime::shared_registry(),
        &definition.headers,
    )
    .await
    {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Model discovery for {provider}: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn models_endpoint(kind: &str, endpoint: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(endpoint).ok()?;
    let path = url.path().trim_end_matches('/');
    let path = match kind {
        "ollama" => format!("{}/api/tags", path),
        "gemini" | "google" => {
            if path.ends_with("/models") {
                path.into()
            } else {
                format!("{path}/models")
            }
        }
        "anthropic" => {
            if let Some(root) = path.strip_suffix("/messages") {
                format!("{root}/models")
            } else if path.ends_with("/v1") {
                format!("{path}/models")
            } else {
                format!("{path}/v1/models")
            }
        }
        "openai" | "openai-compatible" => {
            let root = path
                .strip_suffix("/chat/completions")
                .or_else(|| path.strip_suffix("/responses"))
                .unwrap_or(path);
            format!("{root}/models")
        }
        _ => return None,
    };
    url.set_path(&path);
    Some(url.into())
}

pub(crate) async fn discover(
    kind: &str,
    endpoint: &str,
    key: &str,
    provider: &str,
    registry: &SharedModelRegistry,
) -> Result<Vec<ModelEntry>> {
    discover_with_headers(
        kind,
        endpoint,
        key,
        provider,
        registry,
        &std::collections::BTreeMap::new(),
    )
    .await
}

pub(crate) async fn discover_with_headers(
    kind: &str,
    endpoint: &str,
    key: &str,
    provider: &str,
    registry: &SharedModelRegistry,
    headers: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<ModelEntry>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let initial = reqwest::Url::parse(endpoint).map_err(Error::custom_from_err)?;
    let mut next = initial.clone();
    let mut tokens = std::collections::HashSet::new();
    let mut entries = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let mut request = client.get(next.clone());
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        match kind {
            "anthropic" => {
                request = request
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01")
            }
            "gemini" | "google" => request = request.header("x-goog-api-key", key),
            _ if !key.is_empty() => request = request.bearer_auth(key),
            _ => {}
        }
        let response = tokio::time::timeout_at(deadline, request.send())
            .await
            .map_err(|_| Error::custom("Model discovery exceeded its pagination deadline"))??;
        if !response.status().is_success() {
            let status = response.status();
            return Err(crate::provider_error(
                "Model discovery",
                status,
                &response.text().await.unwrap_or_default(),
            ));
        }
        let body: Value = tokio::time::timeout_at(deadline, response.json())
            .await
            .map_err(|_| Error::custom("Model discovery exceeded its pagination deadline"))??;
        let items = body
            .get("data")
            .or_else(|| body.get("models"))
            .unwrap_or(&body)
            .as_array()
            .ok_or_else(|| Error::custom("Unrecognized model discovery response"))?;
        for item in items {
            if matches!(kind, "gemini" | "google")
                && item["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| !methods.iter().any(|v| v == "generateContent"))
            {
                continue;
            }
            let Some(id) = item["id"].as_str().or_else(|| item["name"].as_str()) else {
                continue;
            };
            let id = if matches!(kind, "gemini" | "google") {
                id.strip_prefix("models/").unwrap_or(id)
            } else {
                id
            };
            if crate::types::validate_model_id(id).is_err() {
                continue;
            }
            let as_u32 = |v: &Value| {
                v.as_u64()
                    .and_then(|n| n.try_into().ok())
                    .filter(|n| *n > 0)
            };
            let tools = item["supported_parameters"]
                .as_array()
                .map(|params| params.iter().any(|v| v == "tools"));
            let metadata = ModelMetadata {
                display_name: item["displayName"]
                    .as_str()
                    .or_else(|| item["display_name"].as_str())
                    .or_else(|| item["name"].as_str())
                    .filter(|name| *name != id && !name.starts_with("models/"))
                    .map(String::from),
                max_tokens: as_u32(&item["outputTokenLimit"])
                    .or_else(|| as_u32(&item["top_provider"]["max_completion_tokens"])),
                context_window: as_u32(&item["inputTokenLimit"])
                    .or_else(|| as_u32(&item["context_length"])),
                tools,
                ..Default::default()
            };
            let full = format!("{provider}/{id}");
            let mut registry = registry.write();
            registry.discovered.insert(full, metadata);
            entries.push(registry.entry(provider, id, true));
        }
        let cursor = body["nextPageToken"]
            .as_str()
            .map(|token| ("pageToken", token))
            .or_else(|| {
                (body["has_more"] == true)
                    .then(|| body["last_id"].as_str().map(|id| ("after_id", id)))
                    .flatten()
            });
        if body["has_more"] == true && cursor.is_none() {
            return Err(Error::custom(
                "Model discovery omitted its pagination cursor",
            ));
        }
        let Some((parameter, token)) = cursor else {
            break;
        };
        if !tokens.insert(token.to_owned()) {
            return Err(Error::custom(
                "Model discovery repeated a pagination cursor",
            ));
        }
        next = initial.clone();
        next.query_pairs_mut().append_pair(parameter, token);
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries.dedup_by(|a, b| a.id == b.id);
    Ok(entries)
}

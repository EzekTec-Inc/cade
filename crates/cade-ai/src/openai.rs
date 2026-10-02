use crate::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{
    CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage,
    provider_error, retry_with_backoff,
};

#[cfg(test)]
use super::clean_openai_schema;

mod responses;
pub(crate) mod schema_normalizer;
pub(crate) mod wire;
pub(crate) use responses::is_continuation as is_responses_continuation;
pub(crate) use schema_normalizer::ToolSchemaNormalizer;
pub(crate) use wire::OpenAiWireEngine;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

// region:    --- Model Capabilities

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    ChatCompletions,
    Responses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenParameter {
    MaxTokens,
    MaxCompletionTokens,
    MaxOutputTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningStrategy {
    None,
    TopLevelReasoningEffort,
    NestedReasoningObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiModelCapabilities {
    pub default_protocol: ApiProtocol,
    pub token_parameter: TokenParameter,
    pub reasoning_strategy: ReasoningStrategy,
    pub is_frontier: bool,
}

impl OpenAiModelCapabilities {
    pub fn for_model(model: &str) -> Self {
        let (provider, bare) = model.split_once('/').unwrap_or(("openai", model));
        let metadata = crate::runtime::shared_registry()
            .read()
            .metadata(provider, bare);
        Self {
            default_protocol: metadata.protocol.unwrap_or(ApiProtocol::ChatCompletions),
            token_parameter: metadata
                .token_parameter
                .unwrap_or(TokenParameter::MaxTokens),
            reasoning_strategy: metadata.reasoning.unwrap_or(ReasoningStrategy::None),
            is_frontier: metadata.preview_gateway.unwrap_or(false),
        }
    }
}

// endregion: --- Model Capabilities

/// Check if a model represents an unreleased/frontier preview model (e.g. gpt-5, gpt-5.5-pro, gpt-5.6, gpt-6).
pub(crate) fn is_frontier_preview_model(model: &str) -> bool {
    OpenAiModelCapabilities::for_model(model).is_frontier
}

#[cfg(test)]
fn needs_max_completion_tokens(model: &str) -> bool {
    OpenAiModelCapabilities::for_model(model).token_parameter == TokenParameter::MaxCompletionTokens
}

#[cfg(test)]
fn is_o_series(model: &str) -> bool {
    let (provider, bare) = model.split_once('/').unwrap_or(("openai", model));
    crate::runtime::RuntimeRegistry::configured()
        .metadata(provider, bare)
        .developer_role
        == Some(true)
}

#[cfg(test)]
fn requires_responses_api_for_tools_with_reasoning(req: &CompletionRequest) -> bool {
    OpenAiModelCapabilities::for_model(&req.model).is_frontier && !req.tools.is_empty()
}

fn map_reasoning_effort(effort: &str) -> Option<&'static str> {
    match effort {
        "xhigh" => Some("high"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "none" => Some("none"),
        _ => None,
    }
}

fn configured_reasoning_effort(
    metadata: &crate::runtime::ModelMetadata,
    effort: &str,
) -> Option<String> {
    match metadata.reasoning_values.as_ref() {
        Some(values) => values.get(effort).cloned(),
        None => map_reasoning_effort(effort).map(String::from),
    }
}

fn parse_tool_arguments(arguments: &str) -> Result<Value> {
    serde_json::from_str(if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    })
    .map_err(Into::into)
}

pub(crate) fn parse_token_usage(usage: &Value, model: &str) -> Option<TokenUsage> {
    let in_tok = usage["prompt_tokens"]
        .as_u64()
        .or_else(|| usage["input_tokens"].as_u64())
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX);
    let out_tok = usage["completion_tokens"]
        .as_u64()
        .or_else(|| usage["output_tokens"].as_u64())
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX);
    let cache_tok = usage["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .or_else(|| usage["input_token_details"]["cached_tokens"].as_u64())
        .or_else(|| usage["input_tokens_details"]["cached_tokens"].as_u64())
        .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX);

    if in_tok > 0 || out_tok > 0 || cache_tok > 0 {
        Some(TokenUsage {
            // Providers include cached tokens in total input tokens, so subtract to get non-cached input.
            input_tokens: in_tok.saturating_sub(cache_tok),
            output_tokens: out_tok,
            cache_read_tokens: cache_tok,
            cache_write_tokens: 0,
            model: crate::catalogue::normalize_model_id_for_lookup(model),
        })
    } else {
        None
    }
}

/// Fetch model IDs from an OpenAI-compatible `/v1/models` endpoint.
///
/// Handles two response shapes:
///   `{ "data": [ { "id": "..." }, … ] }` — OpenAI / Groq / OpenRouter
///   `[ { "id": "..." }, … ]`             — some providers return a bare array
///
/// Returns a sorted `Vec<String>` of model IDs; empty on any error.
/// Compatibility listing API. A model ID alone does not prove chat capability;
/// unclassified IDs remain discoverable instead of applying a name allowlist.
pub async fn fetch_openai_chat_models(api_key: &str) -> Vec<String> {
    crate::discovery::default_models("openai", api_key)
        .await
        .into_iter()
        .filter_map(|entry| entry.id.split_once('/').map(|(_, id)| id.to_owned()))
        .collect()
}

pub async fn fetch_model_ids(models_url: &str, api_key: &str) -> Vec<String> {
    let registry = std::sync::Arc::new(parking_lot::RwLock::new(
        crate::runtime::RuntimeRegistry::default(),
    ));
    match crate::discovery::discover(
        "openai-compatible",
        models_url,
        api_key,
        "gateway",
        &registry,
    )
    .await
    {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|e| e.id.split_once('/').map(|(_, id)| id.to_owned()))
            .collect(),
        Err(e) => {
            tracing::warn!("Model discovery failed: {e}");
            Vec::new()
        }
    }
}

// endregion: --- Tests

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    /// Override base URL for OpenAI-compatible endpoints (e.g. Together, Groq)
    base_url: String,
    provider_name: String,
    label: String,
    models: crate::SharedModelRegistry,
}

#[cfg(test)]
const OPENAI_MAX_TOOLS: usize = 128;
const PRIORITY_TOOL_NAMES: &[&str] = &[
    "load_skill",
    "search_memory",
    "conversation_search",
    "archival_memory_search",
    "update_memory",
    "update_memory_typed",
    "memory_apply_patch",
    "set_plan",
    "UpdatePlan",
    "finish_task",
    "ask_user_question",
    "create_checkpoint",
    "restore_checkpoint",
    "list_checkpoints",
];

const CORE_NATIVE_TOOL_NAMES: &[&str] = &[
    "bash",
    "read_file",
    "write_file",
    "edit_file",
    "apply_patch",
    "glob",
    "grep",
];

fn tool_name(schema: &Value) -> Option<&str> {
    if let Some(name) = schema
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(Value::as_str)
    {
        return Some(name);
    }
    schema.get("name").and_then(Value::as_str)
}

fn schema_bool(schema: &Value, key: &str) -> bool {
    schema
        .get("x-cade")
        .and_then(|metadata| metadata.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn schema_str<'a>(schema: &'a Value, key: &str) -> Option<&'a str> {
    schema
        .get("x-cade")
        .and_then(|metadata| metadata.get(key))
        .and_then(Value::as_str)
}

fn has_tag(schema: &Value, target_tag: &str) -> bool {
    schema
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| tags.iter().any(|tag| tag.as_str() == Some(target_tag)))
        .unwrap_or(false)
}

fn is_meta_tool(schema: &Value) -> bool {
    has_tag(schema, "meta")
        || tool_name(schema).is_some_and(|name| PRIORITY_TOOL_NAMES.contains(&name))
}

fn is_core_native_tool(schema: &Value) -> bool {
    has_tag(schema, "cade")
        && tool_name(schema).is_some_and(|name| CORE_NATIVE_TOOL_NAMES.contains(&name))
}

fn is_core_server_tool(schema: &Value) -> bool {
    schema_bool(schema, "core_server")
        || schema_bool(schema, "is_core")
        || has_tag(schema, "core_mcp")
        || has_tag(schema, "core")
}

fn tool_server_key(schema: &Value) -> &str {
    schema_str(schema, "server_key")
        .or_else(|| tool_name(schema).and_then(|n| n.split_once("__").map(|(prefix, _)| prefix)))
        .unwrap_or("")
}

fn capped_tools_with_limit(schemas: &[Value], limit: usize) -> Vec<&Value> {
    let mut selected: Vec<&Value> = Vec::with_capacity(limit.min(schemas.len()));

    // 1. Tier 0: Meta Tools (Memory, Skills, Planning, Task Lifecycle)
    let mut meta_tools: Vec<&Value> = schemas.iter().filter(|s| is_meta_tool(s)).collect();
    meta_tools.sort_by_key(|s| (tool_server_key(s), tool_name(s).unwrap_or("")));
    selected.extend(meta_tools.into_iter().take(limit));

    if selected.len() >= limit {
        return selected;
    }

    // 2. Tier 0.5: Reserved Core Native Tools (Filesystem, Shell, Editing)
    // Ensures basic coding capabilities are never starved by large MCP server toolsets.
    let mut core_native: Vec<&Value> = schemas
        .iter()
        .filter(|s| !is_meta_tool(s) && is_core_native_tool(s))
        .collect();
    core_native.sort_by_key(|s| tool_name(s).unwrap_or(""));
    let native_slots = limit.saturating_sub(selected.len());
    selected.extend(core_native.into_iter().take(native_slots));

    if selected.len() >= limit {
        return selected;
    }

    // 3. Tier 1: Core MCP Servers (Server-Aware Fair Round-Robin Allocation)
    let mut core_by_server: std::collections::BTreeMap<&str, Vec<&Value>> =
        std::collections::BTreeMap::new();
    for schema in schemas
        .iter()
        .filter(|s| !is_meta_tool(s) && !is_core_native_tool(s) && is_core_server_tool(s))
    {
        let key = tool_server_key(schema);
        core_by_server.entry(key).or_default().push(schema);
    }

    for tools in core_by_server.values_mut() {
        tools.sort_by_key(|s| tool_name(s).unwrap_or(""));
    }

    let mut round = 0;
    loop {
        let mut added_in_round = 0;
        for tools in core_by_server.values() {
            if selected.len() >= limit {
                break;
            }
            if let Some(&tool) = tools.get(round) {
                selected.push(tool);
                added_in_round += 1;
            }
        }
        if added_in_round == 0 || selected.len() >= limit {
            break;
        }
        round += 1;
    }

    if selected.len() >= limit {
        return selected;
    }

    // 4. Tier 2: Remaining Non-Core Tools (Sorted deterministically by server_key, tool_name)
    let mut remaining: Vec<&Value> = schemas
        .iter()
        .filter(|s| !is_meta_tool(s) && !is_core_native_tool(s) && !is_core_server_tool(s))
        .collect();
    remaining.sort_by_key(|s| (tool_server_key(s), tool_name(s).unwrap_or("")));

    let slots_left = limit.saturating_sub(selected.len());
    selected.extend(remaining.into_iter().take(slots_left));

    selected
}

impl OpenAiProvider {
    /// Return a human-readable label for this provider instance.
    /// Used in error messages so users see "OpenRouter" instead of "OpenAI".
    pub(crate) fn provider_label(&self) -> &str {
        &self.label
    }

    /// Resolve the target endpoint for a model request.
    /// Preview gateway eligibility comes from registered/compatibility metadata.
    /// Appending a protocol path preserves any configured gateway query parameters.
    fn append_endpoint_path(base_url: &str, path: &str) -> String {
        let trimmed = base_url.trim().trim_end_matches('/');
        if let Ok(mut url) = reqwest::Url::parse(trimmed) {
            let root = url.path().trim_end_matches('/');
            if root.ends_with("/chat/completions") || root.ends_with("/responses") {
                return url.into();
            }
            let endpoint = format!("{root}{path}");
            url.set_path(&endpoint);
            return url.into();
        }
        if trimmed.ends_with("/chat/completions") || trimmed.ends_with("/responses") {
            trimmed.to_string()
        } else {
            format!("{trimmed}{path}")
        }
    }

    fn resolve_endpoint_with_preview(
        &self,
        model: &str,
        use_responses_api: bool,
        preview_override: Option<&str>,
    ) -> String {
        let endpoint_path = if use_responses_api {
            "/responses"
        } else {
            "/chat/completions"
        };

        if self.base_url == OPENAI_URL && self.metadata(model).preview_gateway == Some(true) {
            let env_opt = std::env::var("OPENAI_PREVIEW_BASE_URL").ok();
            let opt = preview_override.or(env_opt.as_deref());
            if let Some(preview_url) = opt {
                let trimmed = preview_url.trim();
                if !trimmed.is_empty() {
                    return Self::append_endpoint_path(trimmed, endpoint_path);
                }
            }
        }

        if self.base_url == OPENAI_URL && use_responses_api {
            return OPENAI_RESPONSES_URL.to_string();
        }

        Self::append_endpoint_path(&self.base_url, endpoint_path)
    }

    fn protocol_endpoint(&self, req: &CompletionRequest) -> (ApiProtocol, String) {
        let desired = self.protocol(req);
        let endpoint =
            self.resolve_endpoint_with_preview(&req.model, desired == ApiProtocol::Responses, None);
        // Freeze the paired contract before any await. The URL is authoritative
        // even if shared metadata is edited while this request is being prepared.
        (
            Self::endpoint_protocol(&endpoint).unwrap_or(desired),
            endpoint,
        )
    }

    #[cfg(test)]
    fn resolve_endpoint_for_request(&self, req: &CompletionRequest) -> String {
        self.protocol_endpoint(req).1
    }

    pub fn with_registry(
        mut self,
        provider_name: String,
        models: crate::SharedModelRegistry,
    ) -> Self {
        self.provider_name = provider_name;
        self.models = models;
        let definitions = crate::provider_registry::ProviderRegistry::configured();
        if let Some(definition) = definitions.get(&self.provider_name) {
            self.apply_definition(Some(definition));
        } else {
            self.label = self.provider_name.clone();
        }
        self
    }

    pub(crate) fn with_provider_definition(
        mut self,
        definition: &crate::provider_registry::ProviderDef,
    ) -> Self {
        self.apply_definition(Some(definition));
        self
    }

    fn apply_definition(&mut self, definition: Option<&crate::provider_registry::ProviderDef>) {
        self.label = definition
            .and_then(|d| d.display_name.clone())
            .unwrap_or_else(|| self.provider_name.clone());
        self.client = crate::utils::build_provider_http_client(definition);
    }

    fn metadata(&self, model: &str) -> crate::runtime::ModelMetadata {
        let registry = self.models.read();
        registry.metadata(
            &self.provider_name,
            registry.upstream_model(&self.provider_name, model),
        )
    }

    fn protocol(&self, req: &CompletionRequest) -> ApiProtocol {
        if self.base_url == OPENAI_URL
            && self.metadata(&req.model).preview_gateway == Some(true)
            && let Ok(preview) = std::env::var("OPENAI_PREVIEW_BASE_URL")
            && let Some(protocol) = Self::endpoint_protocol(&preview)
        {
            return protocol;
        }
        // An explicitly configured complete Chat URL is a protocol choice. A root
        // gateway URL permits model metadata to choose the paired endpoint.
        if self.base_url != OPENAI_URL
            && let Some(protocol) = Self::endpoint_protocol(&self.base_url)
        {
            return protocol;
        }
        self.metadata(&req.model)
            .protocol
            .unwrap_or(ApiProtocol::ChatCompletions)
    }

    fn endpoint_protocol(endpoint: &str) -> Option<ApiProtocol> {
        let url = reqwest::Url::parse(endpoint).ok()?;
        let path = url.path().trim_end_matches('/');
        if path.ends_with("/responses") {
            Some(ApiProtocol::Responses)
        } else if path.ends_with("/chat/completions") {
            Some(ApiProtocol::ChatCompletions)
        } else {
            None
        }
    }

    fn validate_request(&self, req: &CompletionRequest) -> Result<()> {
        self.validate_model(&req.model)?;
        if !req.tools.is_empty() && self.metadata(&req.model).tools == Some(false) {
            return Err(crate::Error::custom(format!(
                "Model '{}' is registered without tool support",
                req.model
            )));
        }
        if req.max_tokens == 0 {
            return Err(crate::Error::custom("max_tokens must be greater than zero"));
        }
        Ok(())
    }

    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let base = base_url
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("OPENAI_BASE_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| OPENAI_URL.to_string());

        let definitions = crate::provider_registry::ProviderRegistry::configured();
        let definition = definitions
            .get_all_providers()
            .iter()
            .find(|p| p.chat_url == base || p.endpoint() == base);
        let provider_name = definition.map(|p| p.name.clone()).unwrap_or_else(|| {
            if base == OPENAI_URL {
                "openai".into()
            } else {
                "openai-compatible".into()
            }
        });
        let label = definition
            .and_then(|d| d.display_name.clone())
            .unwrap_or_else(|| provider_name.clone());
        Self {
            client: crate::utils::build_provider_http_client(definition),
            api_key,
            base_url: base,
            provider_name,
            label,
            models: crate::runtime::shared_registry(),
        }
    }

    #[cfg(test)]
    fn to_openai_messages(req: &CompletionRequest) -> Value {
        Self::to_openai_messages_with_role(req, is_o_series(&req.model))
    }

    fn to_openai_messages_with_role(req: &CompletionRequest, developer: bool) -> Value {
        let mut combined_system = String::new();
        let mut processed_messages = Vec::new();

        for m in &req.messages {
            if m.role == "system" {
                if !combined_system.is_empty() {
                    combined_system.push_str("\n\n");
                }
                combined_system.push_str(&m.content);
            } else {
                processed_messages.push(m);
            }
        }

        let mut json_messages = Vec::new();

        if !combined_system.is_empty() {
            let role = if developer { "developer" } else { "system" };
            json_messages.push(json!({"role": role, "content": combined_system}));
        }

        for m in processed_messages {
            let value = match m.role.as_str() {
                "tool" => {
                    let tid = m.tool_call_id.as_deref().unwrap_or("");
                    if tid.is_empty() {
                        tracing::warn!("OpenAI: skipping tool message with missing tool_call_id");
                        continue;
                    }
                    json!({
                        "role": "tool",
                        "tool_call_id": tid,
                        "content": m.content
                    })
                }
                "assistant" if m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                    let tcs: Vec<Value> = m.tool_calls.as_deref().unwrap_or_default().iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })).collect();
                    json!({"role": "assistant", "content": m.content, "tool_calls": tcs})
                }
                "assistant" => {
                    json!({"role": "assistant", "content": m.content})
                }
                _ => {
                    // When images are attached, build a multi-part content array.
                    if m.role == "user"
                        && let Some(images) = &m.images
                        && !images.is_empty()
                    {
                        let mut parts: Vec<Value> = images.iter().map(|img| json!({
                            "type": "image_url",
                            "image_url": {"url": format!("data:{};base64,{}", img.media_type, img.data)}
                        })).collect();
                        if !m.content.is_empty() {
                            parts.push(json!({"type": "text", "text": m.content}));
                        }
                        json!({"role": m.role, "content": parts})
                    } else {
                        json!({"role": m.role, "content": m.content})
                    }
                }
            };
            json_messages.push(value);
        }

        json!(json_messages)
    }

    #[cfg(test)]
    pub(crate) fn to_responses_input(req: &CompletionRequest) -> Value {
        Self::to_responses_input_with_role(req, is_o_series(&req.model))
    }

    fn to_responses_input_with_role(req: &CompletionRequest, developer: bool) -> Value {
        let mut combined_system = String::new();
        let mut processed_messages = Vec::new();

        for m in &req.messages {
            if m.role == "system" {
                if !combined_system.is_empty() {
                    combined_system.push_str("\n\n");
                }
                combined_system.push_str(&m.content);
            } else {
                processed_messages.push(m);
            }
        }

        let mut json_items = Vec::new();

        if !combined_system.is_empty() {
            let role = if developer { "developer" } else { "system" };
            json_items.push(json!({"role": role, "content": combined_system}));
        }

        for m in processed_messages {
            match m.role.as_str() {
                "tool" => {
                    let tid = m.tool_call_id.as_deref().unwrap_or("");
                    if !tid.is_empty() {
                        json_items.push(json!({
                            "type": "function_call_output",
                            "call_id": tid,
                            "output": m.content
                        }));
                    }
                }
                "assistant" => {
                    if !m.content.is_empty() {
                        json_items.push(json!({
                            "role": "assistant",
                            "content": m.content
                        }));
                    }
                    if let Some(tool_calls) = &m.tool_calls {
                        for tc in tool_calls {
                            responses::replay(tc, &mut json_items);
                        }
                    }
                }
                _ => {
                    if m.role == "user"
                        && let Some(images) = &m.images
                        && !images.is_empty()
                    {
                        let mut parts: Vec<Value> = images.iter().map(|img| json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", img.media_type, img.data)
                        })).collect();
                        if !m.content.is_empty() {
                            parts.push(json!({"type": "input_text", "text": m.content}));
                        }
                        json_items.push(json!({"role": m.role, "content": parts}));
                    } else {
                        json_items.push(json!({"role": m.role, "content": m.content}));
                    }
                }
            }
        }

        json!(json_items)
    }

    /// Provide clear, actionable diagnostic guidance when upstream returns 404/400 for preview models.
    fn format_upstream_error(
        label: &str,
        status: reqwest::StatusCode,
        text: &str,
        model: &str,
    ) -> crate::Error {
        if status == reqwest::StatusCode::NOT_FOUND && is_frontier_preview_model(model) {
            crate::Error::Provider {
                status: status.as_u16(),
                msg: format!(
                    "{label} returned 404 Not Found for model '{model}'. If you are using a partner/enterprise preview or internal gateway, configure OPENAI_PREVIEW_BASE_URL (e.g. export OPENAI_PREVIEW_BASE_URL=\"https://your-gateway.example.com/v1\"). Upstream details: {text}"
                ),
            }
        } else {
            provider_error(label, status, text)
        }
    }

    pub(crate) fn parse_response(body: &Value) -> CompletionResponse {
        let choice = &body["choices"][0];
        let finish_reason = choice["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();
        let msg = &choice["message"];
        let mut content = msg["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let reasoning_val = msg["reasoning_content"]
            .as_str()
            .or_else(|| msg["reasoning"].as_str())
            .filter(|s| !s.is_empty());
        if let Some(reasoning) = reasoning_val {
            if let Some(c) = &mut content {
                *c = format!("<reasoning>\n{}\n</reasoning>\n\n{}", reasoning, c);
            } else {
                content = Some(format!("<reasoning>\n{}\n</reasoning>", reasoning));
            }
        }
        let tool_calls: Vec<LlmToolCall> = msg["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tc| LlmToolCall {
                id: tc["id"].as_str().unwrap_or("").to_string(),
                name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments: {
                    let arg_str = tc["function"]["arguments"].as_str().unwrap_or("{}").trim();
                    let arg_str = if arg_str.is_empty() { "{}" } else { arg_str };
                    serde_json::from_str(arg_str).unwrap_or_else(|_| json!({}))
                },
                thought_signature: None,
            })
            .collect();
        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
        }
    }

    fn decode_response(body: &Value, protocol: ApiProtocol) -> Result<CompletionResponse> {
        if let Some(error) = body.get("error").filter(|v| !v.is_null()) {
            return Err(crate::Error::custom(format!(
                "Upstream completion error: {error}"
            )));
        }
        OpenAiWireEngine::decode_response(body, protocol)
    }

    async fn send_completion(
        &self,
        req: &CompletionRequest,
        body: Value,
        protocol: ApiProtocol,
        endpoint: String,
    ) -> Result<CompletionResponse> {
        retry_with_backoff(
            "OpenAI::complete",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let endpoint = endpoint.clone();
                let body = body.clone();
                async move {
                    let mut request = self.client.post(endpoint).json(&body);
                    if !self.api_key.is_empty() {
                        request = request.bearer_auth(&self.api_key);
                    }
                    let response = request.send().await?;
                    if !response.status().is_success() {
                        let status = response.status();
                        return Err(Self::format_upstream_error(
                            self.provider_label(),
                            status,
                            &response.text().await.unwrap_or_default(),
                            &req.model,
                        ));
                    }
                    Self::decode_response(&response.json::<Value>().await?, protocol)
                }
            },
        )
        .await
    }

    fn build_tools_with_limit(req: &CompletionRequest, limit: usize) -> Value {
        let tools: Vec<Value> = capped_tools_with_limit(&req.tools, limit)
            .into_iter()
            .map(Self::openai_tool_from_schema)
            .collect();
        json!(tools)
    }

    #[cfg(test)]
    fn build_tools(req: &CompletionRequest) -> Value {
        Self::build_tools_with_limit(req, OPENAI_MAX_TOOLS)
    }

    fn build_responses_tools(req: &CompletionRequest, limit: usize) -> Value {
        let tools: Vec<Value> = capped_tools_with_limit(&req.tools, limit)
            .iter()
            .map(|schema| ToolSchemaNormalizer::normalize(schema, true))
            .collect();
        json!(tools)
    }

    fn openai_tool_from_schema(schema: &Value) -> Value {
        ToolSchemaNormalizer::normalize(schema, false)
    }

    #[cfg(test)]
    fn build_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        self.build_body_for_protocol(req, stream, self.protocol_endpoint(req).0)
    }

    fn build_body_for_protocol(
        &self,
        req: &CompletionRequest,
        stream: bool,
        protocol: ApiProtocol,
    ) -> Value {
        OpenAiWireEngine::build_body(self, req, stream, protocol)
    }

    pub(crate) fn build_chat_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let registry = self.models.read();
        let bare_model_id = registry.upstream_model(&self.provider_name, &req.model);
        drop(registry);
        let metadata = self.metadata(&req.model);
        let token_field = match metadata.token_parameter {
            Some(TokenParameter::MaxCompletionTokens) => "max_completion_tokens",
            _ => "max_tokens",
        };
        let messages =
            Self::to_openai_messages_with_role(req, metadata.developer_role == Some(true));
        let mut body = json!({
            "model": bare_model_id,
            "messages": messages,
            token_field: req.max_tokens,
        });
        if stream {
            body["stream"] = true.into();
            body["stream_options"] = json!({ "include_usage": true });
        }
        if !req.tools.is_empty() && metadata.tools != Some(false) {
            body["tools"] =
                Self::build_tools_with_limit(req, metadata.max_tools.unwrap_or(usize::MAX));
        }
        if metadata.thinking.as_deref() == Some("deepseek") {
            if let Some(effort_str) = req.reasoning_effort.as_deref()
                && let Some(effort) = metadata
                    .reasoning_values
                    .as_ref()
                    .and_then(|values| values.get(effort_str))
            {
                body["thinking"] =
                    json!({ "type": if effort == "none" { "disabled" } else { "enabled" } });
                body["reasoning_effort"] = json!(effort);
            }
        } else if metadata
            .reasoning
            .is_some_and(|r| r != ReasoningStrategy::None)
            && let Some(effort) = req
                .reasoning_effort
                .as_deref()
                .and_then(|effort| configured_reasoning_effort(&metadata, effort))
        {
            body["reasoning_effort"] = effort.into();
        }
        if metadata.include_reasoning == Some(true) && req.reasoning_effort.is_some() {
            body["include_reasoning"] = true.into();
        }
        body
    }

    fn build_responses_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let registry = self.models.read();
        let bare_model_id = registry.upstream_model(&self.provider_name, &req.model);
        drop(registry);
        let metadata = self.metadata(&req.model);
        let input = Self::to_responses_input_with_role(req, metadata.developer_role == Some(true));
        let mut body = json!({
            "model": bare_model_id,
            "input": input,
            "max_output_tokens": req.max_tokens,
            "include": ["reasoning.encrypted_content"],
        });
        if !req.tools.is_empty() && metadata.tools != Some(false) {
            body["tools"] =
                Self::build_responses_tools(req, metadata.max_tools.unwrap_or(usize::MAX));
        }
        if stream {
            body["stream"] = true.into();
        }
        if metadata
            .reasoning
            .is_some_and(|r| r != ReasoningStrategy::None)
            && let Some(effort) = req
                .reasoning_effort
                .as_deref()
                .and_then(|effort| configured_reasoning_effort(&metadata, effort))
        {
            body["reasoning"] = json!({ "effort": effort });
        }
        body
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        self.validate_request(req)?;
        use tracing::Instrument;
        let span = crate::gen_ai_span!("openai", req);

        let (protocol, endpoint) = self.protocol_endpoint(req);
        self.send_completion(
            req,
            self.build_body_for_protocol(req, false, protocol),
            protocol,
            endpoint,
        )
        .instrument(span)
        .await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        self.validate_request(req)?;
        let (protocol, target_url) = self.protocol_endpoint(req);
        let req_model = req.model.clone();
        let body = self.build_body_for_protocol(req, true, protocol);

        let provider_label = self.provider_label();
        let model_name = req.model.clone();

        //NOTE: Stephen Z. Ezekwem -- this is where the api call and response is implemented.
        let resp = retry_with_backoff(
            "OpenAI::stream",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let endpoint = target_url.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
                let label = provider_label;
                let m_name = model_name.clone();
                async move {
                    let mut req = client.post(&endpoint).json(&body);
                    if !api_key.is_empty() {
                        req = req.bearer_auth(&api_key);
                    }
                    let resp = req.send().await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        return Err(Self::format_upstream_error(label, status, &text, &m_name));
                    }
                    Ok(resp)
                }
            },
        )
        .await?;

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = Vec::new();
            // OpenAI streams tool calls with an `index` field to distinguish
            // parallel calls.  Use a BTreeMap keyed by index so multiple
            // tool calls in one turn are accumulated and emitted separately.
            let mut tool_map: std::collections::BTreeMap<usize, (String, String, String)> =
                std::collections::BTreeMap::new();
            let mut finish_emitted = false;
            let mut responses = responses::StreamState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(crate::Error::custom(format!("{e}"))); return; } };
                buf.extend_from_slice(&chunk);

                let mut start = 0;
                while start <= buf.len() {
                    let Some(pos) = buf.get(start..).and_then(|s| s.iter().position(|&b| b == b'\n')) else { break };
                    let end = start + pos;
                    if let Ok(line_str) = std::str::from_utf8(&buf[start..end]) {
                        let line = line_str.trim();
                        if let Some(data) = line.strip_prefix("data:").map(str::trim_start) {
                            if data.is_empty() { start = end + 1; continue; }
                            if data == "[DONE]" {
                        if protocol == ApiProtocol::Responses {
                            yield Err(crate::Error::custom("Responses stream ended before a terminal event")); return;
                        }
                        let remaining: Vec<(String, String, String)> =
                            std::mem::take(&mut tool_map).into_values().collect();
                        for (id, name, args_str) in remaining {
                            if !name.is_empty() {
                                 let args = match parse_tool_arguments(&args_str) { Ok(args) => args, Err(e) => { yield Err(e); return; } };
                                yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                            }
                        }
                        yield Ok(StreamChunk::Done);
                        return;
                    }
                    let v = match serde_json::from_str::<Value>(data) {
                        Ok(value) => value,
                        Err(e) => { yield Err(crate::Error::custom(format!("Invalid completion SSE JSON: {e}"))); return; }
                    };
                    {
                        let event = v["type"].as_str().unwrap_or_default();
                        if matches!(event, "error" | "response.failed" | "response.cancelled")
                            || matches!(v["response"]["status"].as_str(), Some("failed" | "cancelled"))
                            || v.get("error").is_some_and(|e| !e.is_null()) {
                            yield Err(crate::Error::custom(format!("Upstream stream error: {v}"))); return;
                        }
                        if let Err(e) = OpenAiWireEngine::validate_stream_event(event, &v, protocol) {
                            yield Err(e);
                            return;
                        }
                        // 1. Standard Chat Completions SSE schema
                        if let Some(choices) = v.get("choices").and_then(Value::as_array)
                            && let Some(choice0) = choices.first()
                        {
                            let delta = &choice0["delta"];

                            if let Some(text) = delta["content"].as_str()
                                && !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                            let reasoning_val = delta["reasoning_content"]
                                .as_str()
                                .or_else(|| delta["reasoning"].as_str());
                            if let Some(reasoning) = reasoning_val
                                && !reasoning.is_empty() { yield Ok(StreamChunk::Reasoning(reasoning.to_string())); }
                            if let Some(tcs) = delta["tool_calls"].as_array() {
                                for tc in tcs {
                                    // `index` distinguishes parallel tool calls in one stream
                                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                    let entry = tool_map.entry(idx).or_insert_with(|| (String::new(), String::new(), String::new()));
                                    if let Some(id) = tc["id"].as_str() { entry.0 = id.to_string(); }
                                    if let Some(n) = tc["function"]["name"].as_str() { entry.1 = n.to_string(); }
                                    if let Some(a) = tc["function"]["arguments"].as_str() { entry.2.push_str(a); }
                                }
                            }
                            if let Some(reason) = choice0["finish_reason"].as_str() {
                                if matches!(reason, "stop" | "tool_calls") {
                                    // Emit every accumulated tool call in index order
                                    let calls: Vec<(String, String, String)> =
                                        std::mem::take(&mut tool_map).into_values().collect();
                                    for (id, name, args_str) in calls {
                                        if !name.is_empty() {
                                             let args = match parse_tool_arguments(&args_str) { Ok(args) => args, Err(e) => { yield Err(e); return; } };
                                            yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                                        }
                                    }
                                    // Don't return here — OpenAI sends usage in a separate chunk
                                    // before [DONE] when stream_options.include_usage=true.
                                }
                                if !finish_emitted {
                                    yield Ok(StreamChunk::FinishReason(reason.to_string()));
                                    finish_emitted = true;
                                }
                            }
                        }
                        // 2. Modern Responses API / Realtime wire events
                        else if protocol == ApiProtocol::Responses && v.get("type").is_some() {
                            let chunks = match responses.push(&v, &req_model) {
                                Ok(chunks) => chunks,
                                Err(e) => { yield Err(e); return; }
                            };
                            for chunk in chunks {
                                let done = matches!(chunk, StreamChunk::Done);
                                yield Ok(chunk);
                                if done { return; }
                            }
                        } else {
                            // Standard Chat Completions usage chunk (empty choices, top-level usage)
                            let usage_opt = v.get("usage").filter(|u| !u.is_null());
                            if let Some(usage) = usage_opt
                                && let Some(tu) = parse_token_usage(usage, &req_model)
                            {
                                yield Ok(StreamChunk::Usage(tu));
                            }
                        }
                    }
                        }
                        start = end + 1;
                    } else {
                        yield Err(crate::Error::custom("Invalid UTF-8 in completion SSE")); return;
                    }
                }
                if start > 0 {
                    buf.drain(..start);
                }
            }
            if protocol == ApiProtocol::Responses {
                yield Err(crate::Error::custom("Responses stream ended before a terminal event")); return;
            }
            // Chat-compatible byte streams may omit [DONE] — preserve that fallback.
            // so the SSE client doesn't fall back to the blocking endpoint.
            // Also flush any tool calls that arrived without an explicit finish_reason
            // (some OpenAI-compatible providers omit it).
            let remaining: Vec<(String, String, String)> =
                std::mem::take(&mut tool_map).into_values().collect();
            for (id, name, args_str) in remaining {
                if !name.is_empty() {
                    let args = match parse_tool_arguments(&args_str) { Ok(args) => args, Err(e) => { yield Err(e); return; } };
                    yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                }
            }
            yield Ok(StreamChunk::Done);
        };
        Ok(Box::pin(s))
    }

    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        mut schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        use tracing::Instrument;
        let span = crate::gen_ai_span!("openai", req);

        self.validate_request(req)?;
        let fut = async move {
            let (protocol, endpoint) = self.protocol_endpoint(req);
            let native = self.metadata(&req.model).native_structured == Some(true);
            let fallback;
            let body_req = if native {
                req
            } else {
                fallback = crate::types::structured_fallback_request(req, &schema);
                &fallback
            };
            let mut body = self.build_body_for_protocol(body_req, false, protocol);
            // Unknown gateways use JSON parsing fallback; native schema constraints
            // are opt-in metadata rather than inferred from an arbitrary model ID.
            if native {
                crate::utils::enforce_strict_json_schema(&mut schema);
                if protocol == ApiProtocol::Responses {
                    body["text"] = json!({"format": {"type": "json_schema", "name": "structured_output", "strict": true, "schema": schema}});
                } else {
                    body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": schema}});
                }
            }
            let res = self.send_completion(req, body, protocol, endpoint).await?;

            crate::types::parse_structured_text(res.content.as_deref().unwrap_or_default())
        };

        fut.instrument(span).await
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests;

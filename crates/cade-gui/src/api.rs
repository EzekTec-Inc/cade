use js_sys::Reflect;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
    export function copyText(text) {
        if (navigator && navigator.clipboard) {
            navigator.clipboard.writeText(text).catch(err => {
                console.error('Failed to copy text: ', err);
            });
        }
    }
")]
extern "C" {
    pub fn copyText(text: &str);
}
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response, TextDecoder,
};

/// Low-level HTTP request to the CADE server.
pub async fn api_request(
    method: &str,
    path: &str,
    body: Option<&str>,
    api_key: &str,
) -> Result<String, String> {
    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::Cors);

    if let Some(body_str) = body {
        let js_body = JsValue::from_str(body_str);
        opts.set_body(&js_body);
    }

    let request = Request::new_with_str_and_init(path, &opts).map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Authorization", &format!("Bearer {}", api_key))
        .map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{:?}", e))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{:?}", e))?;
    let resp: Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;

    if !resp.ok() {
        let status = resp.status();
        return Err(format!("HTTP error: {}", status));
    }

    let text_value = JsFuture::from(resp.text().map_err(|e| format!("{:?}", e))?)
        .await
        .map_err(|e| format!("{:?}", e))?;
    Ok(text_value.as_string().unwrap_or_default())
}

// ── Typed wrappers ────────────────────────────────────────────────────────

/// Fetch the list of all agents from the server.
pub async fn list_agents(api_key: &str) -> Result<Vec<cade_api_types::AgentInfo>, String> {
    let body = api_request("GET", "/v1/agents", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

/// Fetch messages for a given agent, optionally filtered by conversation.
pub async fn get_messages(
    agent_id: &str,
    api_key: &str,
    conversation_id: Option<&str>,
) -> Result<Vec<cade_api_types::ChatMessage>, String> {
    let path = match conversation_id {
        Some(cid) => format!("/v1/agents/{agent_id}/messages?conversation_id={cid}"),
        None => format!("/v1/agents/{agent_id}/messages"),
    };
    let body = api_request("GET", &path, None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

/// Send a message via the streaming SSE endpoint and call `on_event` for every
/// parsed event.  Returns `Ok(())` after the `[DONE]` sentinel.
pub async fn stream_messages<F>(
    agent_id: &str,
    input: &str,
    api_key: &str,
    conversation_id: Option<&str>,
    cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(cade_api_types::StreamEvent),
{
    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let path = format!("/v1/agents/{agent_id}/messages/stream");
    let mut body_obj = serde_json::json!({ "input": input });
    if let Some(cid) = conversation_id {
        body_obj["conversation_id"] = serde_json::Value::String(cid.to_string());
    }
    let body_str = body_obj.to_string();

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::Cors);
    let js_body = JsValue::from_str(&body_str);
    opts.set_body(&js_body);

    let request = Request::new_with_str_and_init(&path, &opts).map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Authorization", &format!("Bearer {}", api_key))
        .map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{:?}", e))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{:?}", e))?;
    let resp: Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;

    if !resp.ok() {
        return Err(format!("HTTP error: {}", resp.status()));
    }

    let stream = resp.body().ok_or_else(|| "No response body".to_string())?;
    let reader: ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|e| format!("{:?}", e))?;
    let decoder = TextDecoder::new().map_err(|e| format!("{:?}", e))?;

    let mut buffer = String::new();

    loop {
        if let Some(token) = &cancel_token {
            if token.load(std::sync::atomic::Ordering::Acquire) {
                return Err("aborted".to_string());
            }
        }
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{:?}", e))?;

        let done = Reflect::get(&result, &JsValue::from_str("done"))
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);

        if done {
            break;
        }

        let value =
            Reflect::get(&result, &JsValue::from_str("value")).map_err(|e| format!("{:?}", e))?;

        if value.is_null() || value.is_undefined() {
            continue;
        }

        let uint8array: js_sys::Uint8Array = value.dyn_into().map_err(|e| format!("{:?}", e))?;
        let chunk = decoder
            .decode_with_buffer_source(&uint8array.into())
            .map_err(|e| format!("{:?}", e))?;

        buffer.push_str(&chunk);

        // Drain complete SSE events (separated by \n\n)
        while let Some(pos) = buffer.find("\n\n") {
            let event_str = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();
            dispatch_sse_lines(&event_str, &mut on_event)?;
        }
    }

    // Process any remaining data after the stream closes
    dispatch_sse_lines(&buffer, &mut on_event)?;

    Ok(())
}

// ── Conversations ─────────────────────────────────────────────────────────

/// Fetch all conversations for an agent.
pub async fn list_conversations(
    agent_id: &str,
    api_key: &str,
) -> Result<Vec<cade_api_types::ConversationInfo>, String> {
    let path = format!("/v1/agents/{agent_id}/conversations");
    let body = api_request("GET", &path, None, api_key).await?;
    // Server returns { "conversations": [...] }
    let map: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;
    let arr = map["conversations"]
        .as_array()
        .ok_or_else(|| "missing conversations array".to_string())?
        .clone();
    serde_json::from_value(serde_json::Value::Array(arr)).map_err(|e| format!("JSON parse: {e}"))
}

/// Create a new conversation for an agent.
pub async fn create_conversation(
    agent_id: &str,
    title: Option<&str>,
    api_key: &str,
) -> Result<cade_api_types::ConversationInfo, String> {
    let path = format!("/v1/agents/{agent_id}/conversations");
    let mut body = serde_json::json!({});
    if let Some(t) = title {
        body["title"] = serde_json::Value::String(t.to_string());
    }
    let resp = api_request("POST", &path, Some(&body.to_string()), api_key).await?;
    serde_json::from_str(&resp).map_err(|e| format!("JSON parse: {e}"))
}

/// Delete a conversation.
pub async fn delete_conversation(
    agent_id: &str,
    conv_id: &str,
    api_key: &str,
) -> Result<(), String> {
    let path = format!("/v1/agents/{agent_id}/conversations/{conv_id}");
    api_request("DELETE", &path, None, api_key).await?;
    Ok(())
}

// ── Providers ─────────────────────────────────────────────────────────────

/// Fetch all configured providers (API keys are never returned).
pub async fn list_providers(api_key: &str) -> Result<serde_json::Value, String> {
    let body = api_request("GET", "/v1/providers", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

/// Add a new provider.
pub async fn add_provider(
    name: &str,
    kind: &str,
    api_key_val: Option<&str>,
    base_url: Option<&str>,
    api_key: &str,
) -> Result<serde_json::Value, String> {
    let mut body = serde_json::json!({ "name": name, "kind": kind });
    if let Some(k) = api_key_val {
        body["api_key"] = serde_json::Value::String(k.to_string());
    }
    if let Some(u) = base_url {
        body["base_url"] = serde_json::Value::String(u.to_string());
    }
    let resp = api_request("POST", "/v1/providers", Some(&body.to_string()), api_key).await?;
    serde_json::from_str(&resp).map_err(|e| format!("JSON parse: {e}"))
}

/// Remove a provider by name.
pub async fn remove_provider(name: &str, api_key: &str) -> Result<(), String> {
    let path = format!("/v1/providers/{name}");
    api_request("DELETE", &path, None, api_key).await?;
    Ok(())
}

/// Fetch provider presets (built-in shortcuts like openrouter, groq, etc.).
pub async fn list_presets(api_key: &str) -> Result<serde_json::Value, String> {
    let body = api_request("GET", "/v1/providers/presets", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

// ── Agents ────────────────────────────────────────────────────────────────

/// Fetch a single agent by ID.
#[allow(dead_code)]
pub async fn get_agent(agent_id: &str, api_key: &str) -> Result<cade_api_types::AgentInfo, String> {
    let path = format!("/v1/agents/{agent_id}");
    let body = api_request("GET", &path, None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

// ── Memory blocks ─────────────────────────────────────────────────────────

/// List memory blocks for an agent.
pub async fn list_memory_blocks(
    agent_id: &str,
    api_key: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let path = format!("/v1/agents/{agent_id}/memory");
    let body = api_request("GET", &path, None, api_key).await?;
    let map: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;
    let arr = map["blocks"]
        .as_array()
        .ok_or_else(|| "missing blocks array".to_string())?
        .clone();
    Ok(arr)
}

// ── Tools / MCP ───────────────────────────────────────────────────────────

/// List all registered tools.
#[allow(dead_code)]
pub async fn list_tools(api_key: &str) -> Result<serde_json::Value, String> {
    let body = api_request("GET", "/v1/tools", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

/// List MCP servers and their exposed tools.
pub async fn list_mcp_servers(api_key: &str) -> Result<Vec<serde_json::Value>, String> {
    let body = api_request("GET", "/v1/mcp", None, api_key).await?;
    let map: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;
    let arr = map["servers"]
        .as_array()
        .ok_or_else(|| "missing servers array".to_string())?
        .clone();
    Ok(arr)
}

// ── Models ────────────────────────────────────────────────────────────────

/// List available models from all configured providers.
pub async fn list_models(api_key: &str) -> Result<serde_json::Value, String> {
    let body = api_request("GET", "/v1/models", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

// ── Metrics / Context Stats ───────────────────────────────────────────────

/// Fetch agent metrics (token usage, costs, etc.).
pub async fn get_metrics(agent_id: &str, api_key: &str) -> Result<serde_json::Value, String> {
    let path = format!("/v1/agents/{agent_id}/metrics");
    let body = api_request("GET", &path, None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

/// Fetch agent context telemetry (budget, tokens, turns).
pub async fn get_context_stats(agent_id: &str, api_key: &str) -> Result<serde_json::Value, String> {
    let path = format!("/v1/agents/{agent_id}/context_stats");
    let body = api_request("GET", &path, None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

// ── Server Config ─────────────────────────────────────────────────────────

/// Fetch server configuration (provider, default_model, version).
pub async fn get_config(api_key: &str) -> Result<serde_json::Value, String> {
    let body = api_request("GET", "/v1/config", None, api_key).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))
}

// ── Events ────────────────────────────────────────────────────────────────

/// List events for an agent.
pub async fn list_events(agent_id: &str, api_key: &str) -> Result<Vec<serde_json::Value>, String> {
    let path = format!("/v1/agents/{agent_id}/events?limit=50");
    let body = api_request("GET", &path, None, api_key).await?;
    let map: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;
    let arr = map["events"]
        .as_array()
        .ok_or_else(|| "missing events array".to_string())?
        .clone();
    Ok(arr)
}

/// Parse `data: ...` lines from a single SSE event block and dispatch to `on_event`.
fn dispatch_sse_lines<F>(block: &str, on_event: &mut F) -> Result<(), String>
where
    F: FnMut(cade_api_types::StreamEvent),
{
    for line in block.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let trimmed = data.trim();
            if trimmed == "[DONE]" {
                return Ok(()); // signal end — let caller decide what to do
            }
            if let Ok(event) = serde_json::from_str::<cade_api_types::StreamEvent>(trimmed) {
                on_event(event);
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct CadeApiClient {
    pub api_key: String,
}

#[allow(dead_code)]
impl CadeApiClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub async fn list_agents(&self) -> Result<Vec<cade_api_types::AgentInfo>, String> {
        let res = api_request("GET", "/v1/agents", None, &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn list_conversations(
        &self,
        agent_id: &str,
    ) -> Result<Vec<cade_api_types::ConversationInfo>, String> {
        let path = format!("/v1/agents/{}/conversations", agent_id);
        let res = api_request("GET", &path, None, &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn create_conversation(
        &self,
        agent_id: &str,
        title: Option<&str>,
    ) -> Result<cade_api_types::ConversationInfo, String> {
        create_conversation(agent_id, title, &self.api_key).await
    }

    pub async fn delete_conversation(&self, agent_id: &str, conv_id: &str) -> Result<(), String> {
        delete_conversation(agent_id, conv_id, &self.api_key).await
    }

    pub async fn get_messages(
        &self,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<Vec<cade_api_types::ChatMessage>, String> {
        get_messages(agent_id, &self.api_key, conversation_id).await
    }

    pub async fn stream_messages<F>(
        &self,
        agent_id: &str,
        input: &str,
        conversation_id: Option<&str>,
        cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        on_event: F,
    ) -> Result<(), String>
    where
        F: FnMut(cade_api_types::StreamEvent),
    {
        stream_messages(
            agent_id,
            input,
            &self.api_key,
            conversation_id,
            cancel_token,
            on_event,
        )
        .await
    }

    pub async fn list_providers(&self) -> Result<serde_json::Value, String> {
        list_providers(&self.api_key).await
    }

    pub async fn add_provider(
        &self,
        name: &str,
        kind: &str,
        api_key_val: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        add_provider(name, kind, api_key_val, base_url, &self.api_key).await
    }

    pub async fn remove_provider(&self, name: &str) -> Result<(), String> {
        remove_provider(name, &self.api_key).await
    }

    pub async fn list_presets(&self) -> Result<serde_json::Value, String> {
        list_presets(&self.api_key).await
    }

    pub async fn list_memory_blocks(
        &self,
        agent_id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        list_memory_blocks(agent_id, &self.api_key).await
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<serde_json::Value>, String> {
        list_mcp_servers(&self.api_key).await
    }

    pub async fn list_models(&self) -> Result<serde_json::Value, String> {
        list_models(&self.api_key).await
    }

    pub async fn get_metrics(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        get_metrics(agent_id, &self.api_key).await
    }

    pub async fn get_context_stats(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        get_context_stats(agent_id, &self.api_key).await
    }

    pub async fn get_config(&self) -> Result<serde_json::Value, String> {
        get_config(&self.api_key).await
    }

    pub async fn list_events(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        list_events(agent_id, &self.api_key).await
    }

    pub async fn list_checkpoints(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let path = format!("/v1/agents/{}/checkpoints", agent_id);
        let res = api_request("GET", &path, None, &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn restore_checkpoint(
        &self,
        agent_id: &str,
        cp_id: &str,
    ) -> Result<serde_json::Value, String> {
        let path = format!("/v1/agents/{}/checkpoints/{}/restore", agent_id, cp_id);
        let res = api_request("POST", &path, None, &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn list_approvals(&self) -> Result<serde_json::Value, String> {
        let res = api_request("GET", "/v1/approvals", None, &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn action_approval(
        &self,
        id: &str,
        action: &str,
    ) -> Result<serde_json::Value, String> {
        let path = format!("/v1/approvals/{}/action", id);
        let body = serde_json::json!({ "action": action });
        let res = api_request("POST", &path, Some(&body.to_string()), &self.api_key).await?;
        serde_json::from_str(&res).map_err(|e| e.to_string())
    }

    pub async fn listen_global_events<F>(&self, on_event: F) -> Result<(), String>
    where
        F: FnMut(serde_json::Value),
    {
        listen_global_events(&self.api_key, on_event).await
    }
}

/// Listen to the global SSE event stream and call `on_event` for each parsed JSON packet.
pub async fn listen_global_events<F>(api_key: &str, mut on_event: F) -> Result<(), String>
where
    F: FnMut(serde_json::Value),
{
    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let path = "/v1/events";

    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let request = Request::new_with_str_and_init(path, &opts).map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Authorization", &format!("Bearer {}", api_key))
        .map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{:?}", e))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{:?}", e))?;
    let resp: Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;

    if !resp.ok() {
        return Err(format!("HTTP error: {}", resp.status()));
    }

    let stream = resp.body().ok_or_else(|| "No response body".to_string())?;
    let reader: ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|e| format!("{:?}", e))?;
    let decoder = TextDecoder::new().map_err(|e| format!("{:?}", e))?;

    let mut buffer = String::new();

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{:?}", e))?;

        let done = Reflect::get(&result, &JsValue::from_str("done"))
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);

        if done {
            break;
        }

        let value =
            Reflect::get(&result, &JsValue::from_str("value")).map_err(|e| format!("{:?}", e))?;

        if value.is_null() || value.is_undefined() {
            continue;
        }

        let uint8array: js_sys::Uint8Array = value.dyn_into().map_err(|e| format!("{:?}", e))?;
        let chunk = decoder
            .decode_with_buffer_source(&uint8array.into())
            .map_err(|e| format!("{:?}", e))?;

        buffer.push_str(&chunk);

        while let Some(pos) = buffer.find("\n\n") {
            let event_str = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();
            
            for line in event_str.lines() {
                if let Some(data_str) = line.strip_prefix("data:") {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(data_str.trim()) {
                        on_event(val);
                    }
                }
            }
        }
    }

    Ok(())
}

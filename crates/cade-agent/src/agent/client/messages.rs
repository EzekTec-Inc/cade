use super::*;

impl CadeClient {

    // -- Messages

    /// Send a user message and return the response messages.
    /// Set `ephemeral=true` for system-injected messages that should not be persisted.
    pub async fn send_message(
        &self,
        agent_id: &str,
        input: &str,
        ephemeral: bool,
    ) -> Result<Vec<CadeMessage>> {
        let mut req = json!({ "input": input });
        if ephemeral {
            req["ephemeral"] = true.into();
        }
        self.post_messages(agent_id, &req).await
    }

    /// Like `send_message` but also attaches base64-encoded images.
    ///
    /// Each element of `images` must be `{"media_type": "image/png", "data": "<b64>"}`.
    pub async fn send_message_with_images(
        &self,
        agent_id: &str,
        input: &str,
        images: Vec<serde_json::Value>,
        ephemeral: bool,
    ) -> Result<Vec<CadeMessage>> {
        let mut req = json!({ "input": input });
        if ephemeral {
            req["ephemeral"] = true.into();
        }
        if !images.is_empty() {
            req["images"] = serde_json::Value::Array(images);
        }
        self.post_messages(agent_id, &req).await
    }

    /// Fetch the most recent assistant message (if any) for an agent.
    pub async fn last_assistant_message(
        &self,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>> {
        let mut req = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/messages/latest")))
            .header("Authorization", format!("Bearer {}", self.api_key));
        if let Some(conv) = conversation_id {
            req = req.query(&[("conversation_id", conv)]);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "last_assistant_message failed: {}",
                text
            )));
        }
        let body: Value = resp.json().await?;
        Ok(body.get("message").cloned())
    }

    // -- Conversations

    pub async fn list_conversations(&self, agent_id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/conversations")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_conversations failed {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body["conversations"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// Fetch messages for an agent, optionally filtered by conversation_id.
    /// Pass an empty string for `conversation_id` to fetch legacy (no-conversation) messages.
    pub async fn get_conversation_messages(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let mut req = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/messages")))
            .header("Authorization", format!("Bearer {}", self.api_key));
        if !conversation_id.is_empty() {
            req = req.query(&[("conversation_id", conversation_id)]);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_conversation_messages failed {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body["messages"].as_array().cloned().unwrap_or_default())
    }

    pub async fn create_conversation(
        &self,
        agent_id: &str,
        title: &str,
    ) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/conversations")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "title": title }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_conversation failed: {txt}"
            )));
        }
        Ok(resp.json().await?)
    }

    pub async fn delete_conversation(&self, agent_id: &str, conv_id: &str) -> Result<()> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}/conversations/{conv_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "delete_conversation failed: {txt}"
            )));
        }
        Ok(())
    }

    // -- Runs (background mode)

    pub async fn get_run(&self, run_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url(&format!("/runs/{run_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_run failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Resume a background run from a given seq_id, streaming events via SSE.
    /// Calls `on_event` for each replayed event, returns full list.
    pub async fn resume_run<F>(
        &self,
        run_id: &str,
        after_seq: i64,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let url = self.url(&format!("/runs/{run_id}/stream"));
        let request = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("starting_after", after_seq.to_string())]);

        let mut es = EventSource::new(request)
            .map_err(|e| crate::Error::custom(format!("EventSource: {e}")))?;
        let mut messages = Vec::new();

        while let Some(event) = es.next().await {
            match event {
                Ok(reqwest_eventsource::Event::Open) => {}
                Ok(reqwest_eventsource::Event::Message(msg)) => {
                    let data = msg.data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        es.close();
                        break;
                    }
                    if let Ok(lm) = serde_json::from_str::<CadeMessage>(data) {
                        on_event(&lm);
                        messages.push(lm);
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(_) => {
                    es.close();
                    break;
                }
            }
        }
        Ok(messages)
    }

    // -- Messages

    /// Stream a user message using SSE. Calls `on_event` for each message as
    /// it arrives (for live rendering), and returns the full collected list.
    ///
    /// Uses `/v1/agents/{id}/messages/stream` — falls back to `send_message`
    /// if the stream endpoint returns a non-2xx status.
    pub async fn stream_message<F>(
        &self,
        agent_id: &str,
        input: &str,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.stream_message_cancellable(agent_id, input, None, false, None, on_event, None)
            .await
    }

    /// Like `stream_message` but checks an optional cancel flag before each SSE event.
    pub async fn stream_message_cancellable<F>(
        &self,
        agent_id: &str,
        input: &str,
        conversation_id: Option<&str>,
        // When true, server skips persisting the user message — for system-injected
        // re-prompts that should not appear in conversation history.
        ephemeral: bool,
        reasoning_effort: Option<&str>,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.stream_message_cancellable_with_images(
            agent_id,
            input,
            conversation_id,
            ephemeral,
            vec![],
            reasoning_effort,
            on_event,
            cancel,
        )
        .await
    }

    /// Like `stream_message_cancellable` but also attaches base64-encoded images.
    pub async fn stream_message_cancellable_with_images<F>(
        &self,
        agent_id: &str,
        input: &str,
        conversation_id: Option<&str>,
        ephemeral: bool,
        images: Vec<serde_json::Value>,
        reasoning_effort: Option<&str>,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let url = self.url(&format!("/agents/{agent_id}/messages/stream"));
        let mut body = json!({ "input": input });
        if let Some(cid) = conversation_id {
            body["conversation_id"] = cid.into();
        }
        if ephemeral {
            body["ephemeral"] = true.into();
        }
        if !images.is_empty() {
            body["images"] = serde_json::Value::Array(images);
        }
        if let Some(effort) = reasoning_effort {
            body["reasoning_effort"] = effort.into();
        }

        let request = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        let mut es = EventSource::new(request)
            .map_err(|e| crate::Error::custom(format!("EventSource: {e}")))?;

        let mut messages: Vec<CadeMessage> = Vec::new();

        while let Some(event) = es.next().await {
            // Check cancel flag on every event (fired ~per token while streaming)
            if cancel.is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst)) {
                es.close();
                return Err(crate::Error::custom("__cancelled__"));
            }
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(msg)) => {
                    let data = msg.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        // Explicitly close to prevent reqwest_eventsource from
                        // auto-reconnecting (SSE spec default), which would re-POST
                        // the same body and duplicate messages on the server.
                        es.close();
                        break;
                    }
                    // Check for server-side error events (e.g. LLM 404/5xx).
                    // The server emits {"error":"..."} as a proper SSE event so
                    // we can show the real error without falling back to /messages
                    // (which would re-persist the user message → duplicate in DB).
                    if let Ok(v) = serde_json::from_str::<Value>(data)
                        && let Some(err_msg) = v["error"].as_str()
                    {
                        es.close();
                        return Err(crate::Error::custom(err_msg));
                    }
                    match serde_json::from_str::<CadeMessage>(data) {
                        Ok(lm) => {
                            on_event(&lm);
                            messages.push(lm);
                        }
                        Err(_) => {
                            // Try parsing as a wrapper object with a messages array
                            if let Ok(v) = serde_json::from_str::<Value>(data)
                                && let Some(arr) = v["messages"].as_array()
                            {
                                for item in arr {
                                    if let Ok(lm) =
                                        serde_json::from_value::<CadeMessage>(item.clone())
                                    {
                                        on_event(&lm);
                                        messages.push(lm);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(reqwest_eventsource::Error::InvalidStatusCode(status, _)) => {
                    // Server returned a non-200 HTTP status (e.g. 401, 404, 502).
                    // DON'T fall back to /messages — that would re-persist the user
                    // message and call the same failing LLM again.
                    // After Fix 3 (messages.rs), CADE's own server returns a proper
                    // SSE error stream instead of 502, so this path is only hit when
                    // connecting to external/legacy servers that return raw HTTP errors.
                    es.close();
                    return Err(crate::Error::custom(format!(
                        "Server returned HTTP {status}"
                    )));
                }
                Err(e) => {
                    // Network / transport errors (connection refused, timeout, etc.).
                    // Fall back to the blocking endpoint only for transport errors —
                    // these typically mean the SSE endpoint is unavailable but the
                    // server itself might still respond to regular POST.
                    tracing::debug!("SSE transport error: {e}, falling back to send_message");
                    es.close();
                    let fallback = self.send_message(agent_id, input, ephemeral).await?;
                    for lm in &fallback {
                        on_event(lm);
                    }
                    return Ok(fallback);
                }
            }
        }

        Ok(messages)
    }

    /// Send a tool result back to the agent after local execution
    pub async fn send_tool_return(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
    ) -> Result<Vec<CadeMessage>> {
        let req = json!({
            "role": "tool",
            "tool_return": {
                "tool_call_id": tool_call_id,
                "content": output,
                "status": if is_error { "error" } else { "success" }
            }
        });
        self.post_messages(agent_id, &req).await
    }

    /// Stream a tool return response (same as send_tool_return but with live events)
    pub async fn stream_tool_return<F>(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.stream_tool_return_cancellable(
            agent_id,
            tool_call_id,
            output,
            is_error,
            None,
            None,
            on_event,
            None,
        )
        .await
    }

    /// Like `stream_tool_return` but checks an optional cancel flag between SSE events.
    pub async fn stream_tool_return_cancellable<F>(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
        conversation_id: Option<&str>,
        reasoning_effort: Option<&str>,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let mut body = json!({
            "role": "tool",
            "tool_return": {
                "tool_call_id": tool_call_id,
                "content": output,
                "status": if is_error { "error" } else { "success" }
            }
        });
        if let Some(effort) = reasoning_effort {
            body["reasoning_effort"] = effort.into();
        }
        if let Some(cid) = conversation_id {
            body["conversation_id"] = cid.into();
        }
        let url = self.url(&format!("/agents/{agent_id}/messages/stream"));
        let request = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        let mut es = EventSource::new(request)
            .map_err(|e| crate::Error::custom(format!("EventSource: {e}")))?;
        let mut messages = Vec::new();

        // Do NOT check cancel on Event::Open.  By the time Event::Open fires the
        // tool-return HTTP POST has already been delivered to the server; the
        // agent is generating its response.  Any residual cancel_turn flag left by
        // the approval modal or I-01 Enter key would silently kill that response
        // before a single byte of content is received.  We begin honouring cancel
        // only on the first actual Message event so the user can still press Esc to
        // abort mid-stream once content starts arriving.
        let mut opened = false;

        while let Some(event) = es.next().await {
            if opened && cancel.is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst)) {
                es.close();
                return Err(crate::Error::custom("__cancelled__"));
            }
            match event {
                Ok(Event::Open) => {
                    opened = true;
                    // Clear any cancel flag that accumulated before the connection
                    // was established (stale approval-modal Enter, buffered Esc,
                    // prior SIGINT, etc.).  The tool result was already POSTed to
                    // the server — the agent's response MUST arrive.  Any cancel
                    // after this point is intentional (user presses Esc mid-stream).
                    if let Some(c) = cancel {
                        c.store(false, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                Ok(Event::Message(msg)) => {
                    let data = msg.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        // Close explicitly — prevents SSE auto-reconnect which would
                        // re-POST the tool_return body and duplicate the DB record.
                        es.close();
                        break;
                    }
                    if let Ok(lm) = serde_json::from_str::<CadeMessage>(data) {
                        on_event(&lm);
                        messages.push(lm);
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(reqwest_eventsource::Error::InvalidStatusCode(status, _)) => {
                    es.close();
                    return Err(crate::Error::custom(format!(
                        "Server returned HTTP {status}"
                    )));
                }
                Err(_) => {
                    // Fallback to non-streaming
                    es.close();
                    let fallback = self
                        .send_tool_return(agent_id, tool_call_id, output, is_error)
                        .await?;
                    for lm in &fallback {
                        on_event(lm);
                    }
                    return Ok(fallback);
                }
            }
        }
        Ok(messages)
    }

    async fn post_messages(&self, agent_id: &str, body: &Value) -> Result<Vec<CadeMessage>> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/messages")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            // Extract human-readable detail from {"detail":"..."} wrapper if present
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["detail"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(crate::Error::custom(msg.to_string()));
        }

        let raw: Value = resp.json().await?;
        let msgs = raw["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                serde_json::from_value(v).unwrap_or(CadeMessage {
                    id: None,
                    message_type: None,
                    data: json!({}),
                })
            })
            .collect();
        Ok(msgs)
    }

}

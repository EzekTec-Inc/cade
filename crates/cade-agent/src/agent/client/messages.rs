use super::*;

impl HttpTransport {
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

    pub async fn create_conversation_fork(
        &self,
        agent_id: &str,
        title: &str,
        parent_id: &str,
    ) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/conversations")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "title": title, "parent_id": parent_id }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_conversation_fork failed: {txt}"
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

    /// Start a server-owned agent run and render its ordered runtime events.
    ///
    /// The server owns context construction, policy, model calls, tool execution,
    /// persistence, and cancellation. Clients only render this event stream.
    pub async fn start_run<F>(
        &self,
        agent_id: &str,
        input: &str,
        conversation_id: Option<&str>,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.start_run_cancellable(agent_id, input, conversation_id, on_event, None)
            .await
    }

    /// Start a server-owned run and translate a presentation cancellation flag
    /// into the runtime's durable cancellation command.
    pub async fn start_run_cancellable<F>(
        &self,
        agent_id: &str,
        input: &str,
        conversation_id: Option<&str>,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let mut body = json!({ "input": input });
        if let Some(conversation_id) = conversation_id {
            body["conversation_id"] = conversation_id.into();
        }
        self.consume_run_stream(
            EventSource::new(
                self.client
                    .post(self.url(&format!("/agents/{agent_id}/run")))
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .json(&body),
            )
            .map_err(|error| crate::Error::custom(format!("EventSource: {error}")))?,
            agent_id,
            conversation_id,
            Vec::new(),
            on_event,
            cancel,
        )
        .await
    }

    /// Request cancellation of a server-owned run.
    pub async fn cancel_run(&self, run_id: &str) -> Result<serde_json::Value> {
        let response = self
            .client
            .post(self.url(&format!("/runs/{run_id}/cancel")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(crate::Error::custom(format!(
                "cancel_run failed {}",
                response.status()
            )));
        }
        Ok(response.json().await?)
    }

    // -- Resilient run streaming

    /// Max seconds to keep retrying a dropped run stream before giving up.
    /// Long-running tasks are expected to legitimately run for hours; a 12 hour
    /// accounting window is effectively "don't give up during a workday".
    const RUN_STREAM_RETRY_WINDOW_SECS: u64 = 12 * 60 * 60;
    /// Upper bound on the exponential reconnect backoff (open-ended retries).
    const RUN_STREAM_MAX_BACKOFF_MS: u64 = 10_000;

    /// Poll `GET /v1/runs/{run_id}/stream?starting_after=` and return the data
    /// payloads.  Used by the resume path (plain reqwest — no auto-reconnect,
    /// no SSE event-source state to leak across retries).
    async fn poll_run_events(&self, run_id: &str, after_seq: i64) -> Result<Vec<String>> {
        let url = self.url(&format!("/runs/{run_id}/stream"));
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("starting_after", after_seq.to_string())])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "run stream replay failed: {}",
                resp.status()
            )));
        }
        let body = resp.text().await?;
        Ok(parse_sse_data(&body))
    }

    /// Fetch the current status of a run (`GET /v1/runs/{id}`).  `None` when
    /// the run no longer exists or the status endpoint is unreachable (treated
    /// as "still busy — keep polling" by the resume loop).
    async fn run_status(&self, run_id: &str) -> Option<String> {
        let url = self.url(&format!("/runs/{run_id}"));
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        body["status"].as_str().map(str::to_owned)
    }

    /// If the SSE handshake dropped before any event delivered a `run_id`,
    /// the server may still have accepted the run (it processes independently
    /// of the client connection).  Recover the run id from the agent's recent
    /// runs rather than erroring out or — worse — re-POSTing a duplicate run.
    async fn discover_latest_run_id(
        &self,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Option<String> {
        let url = self.url(&format!("/agents/{agent_id}/runs"));
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .ok()?;
        let body: serde_json::Value = resp.json().await.ok()?;
        body["runs"]
            .as_array()?
            .iter()
            .filter(|r| {
                conversation_id.is_none_or(|cid| r["conversation_id"].as_str() == Some(cid))
            })
            .filter(|r| {
                let status = r["status"].as_str().unwrap_or("");
                matches!(status, "running" | "cancelling")
            })
            .max_by(|a, b| {
                let ka = a["created_at"].as_str().unwrap_or("");
                let kb = b["created_at"].as_str().unwrap_or("");
                ka.cmp(kb)
            })
            .and_then(|r| r["id"].as_str().map(String::from))
    }

    /// Resume an interrupted run by backfilling the durable event log.  The
    /// server persists every event, so a dropped transport connection loses no
    /// data: we replay `seq > after_seq` (deduped), then follow the run to
    /// completion (polling the durable log until a `run_done` envelope or a
    /// terminal status appears).  Retries with capped exponential backoff.
    async fn backfill_follow_run<F>(
        &self,
        run_id: &str,
        mut after_seq: i64,
        mut messages: Vec<CadeMessage>,
        on_event: &F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(Self::RUN_STREAM_RETRY_WINDOW_SECS);
        let mut attempt: u64 = 0;

        loop {
            // Honour a user cancellation request while offline: fire the
            // durable cancel endpoint so the server stops work, then surface
            // cancellation like the live path does.
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst)) {
                let _ = self.cancel_run(run_id).await;
                return Err(crate::Error::custom("__cancelled__"));
            }

            if std::time::Instant::now() >= deadline {
                return Err(crate::Error::custom(format!(
                    "run {run_id} is still running but unreachable after {} min — reconnect and resume with the same prompt to pick it up",
                    Self::RUN_STREAM_RETRY_WINDOW_SECS / 60,
                )));
            }

            match self.poll_run_events(run_id, after_seq).await {
                Ok(payloads) => {
                    let mut completed = false;
                    let mut saw_any = false;
                    for payload in payloads {
                        if payload == "[DONE]" {
                            // Replay snapshot exhausted — keep following.
                            continue;
                        }
                        let Ok(message) = serde_json::from_str::<CadeMessage>(&payload) else {
                            continue;
                        };
                        if let Some(seq) = message.seq_id() {
                            if seq <= after_seq {
                                // Defensive dedup: replayed/duplicated event.
                                continue;
                            }
                            after_seq = seq;
                            saw_any = true;
                        }
                        if message.msg_type() == "run_done" {
                            completed = true;
                        }
                        on_event(&message);
                        messages.push(message);
                    }

                    if completed {
                        return Ok(messages);
                    }

                    // If the log is quiescent, confirm terminal state via the
                    // run resource (covers the ordering race where the run
                    // status flips before the run_done event is persisted).
                    if !saw_any
                        && let Some(status) = self.run_status(run_id).await
                        && matches!(status.as_str(), "done" | "error" | "cancelled")
                    {
                        return Ok(messages);
                    }
                    attempt = 0;
                }
                Err(error) => {
                    // Transient network/HTTP failure — back off and retry.
                    // Keep the previous last_seq; the durable log has it all.
                    tracing::debug!(run_id = %run_id, %error, "run stream replay failed, retrying");
                }
            }

            let delay_ms = Self::RUN_STREAM_MAX_BACKOFF_MS
                .min(250u64.saturating_mul(2_u64.saturating_pow(attempt.min(5) as u32)));
            attempt += 1;
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    }

    async fn consume_run_stream<F>(
        &self,
        mut events: EventSource,
        agent_id: &str,
        conversation_id: Option<&str>,
        mut messages: Vec<CadeMessage>,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let mut run_id = None;
        let mut last_seq_id: i64 = -1;
        let mut cancellation_requested = false;

        let deliver = |message: &CadeMessage,
                       messages: &mut Vec<CadeMessage>,
                       run_id: &mut Option<String>,
                       last_seq_id: &mut i64| {
            if run_id.is_none() {
                *run_id = message.run_id().map(str::to_owned);
            }
            if let Some(seq) = message.seq_id() {
                let current = *last_seq_id;
                *last_seq_id = current.max(seq);
            }
            on_event(message);
            messages.push(message.clone());
        };

        while let Some(event) = events.next().await {
            if !cancellation_requested
                && cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
                && let Some(id) = run_id.as_deref()
            {
                self.cancel_run(id).await?;
                cancellation_requested = true;
            }
            match event {
                Ok(reqwest_eventsource::Event::Open) => {}
                Ok(reqwest_eventsource::Event::Message(message)) => {
                    let data = message.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        events.close();
                        break;
                    }
                    let message: CadeMessage = serde_json::from_str(data).map_err(|error| {
                        crate::Error::custom(format!("invalid run event: {error}"))
                    })?;
                    deliver(&message, &mut messages, &mut run_id, &mut last_seq_id);
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(error) => {
                    events.close();
                    tracing::warn!("SSE run transport error: {error:?}");

                    // Transparent connection recovery (opencode-style): the
                    // server persists every event durably and keeps running
                    // independently of the client connection, so a dropped SSE
                    // body loses nothing.  Resume from the last observed
                    // sequence id by backfilling the durable log and following
                    // the run to completion.  If the handshake died before any
                    // event carried a run_id, the run may still have been
                    // accepted — recover it from the agent's recent runs rather
                    // than erroring out or re-POSTing a duplicate.
                    let id = match run_id.clone() {
                        Some(id) => Some(id),
                        None => self.discover_latest_run_id(agent_id, conversation_id).await,
                    };
                    if !cancellation_requested && let Some(id) = id {
                        return self
                            .backfill_follow_run(&id, last_seq_id, messages, &on_event, cancel)
                            .await;
                    }

                    return Err(crate::Error::custom(error.to_string()));
                }
            }
        }
        Ok(messages)
    }

    /// Resume a run that is still active from a given seq_id.
    /// Backfills the durable event log and follows the run to completion.
    pub async fn resume_run<F>(
        &self,
        run_id: &str,
        after_seq: i64,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.backfill_follow_run(run_id, after_seq, Vec::new(), &on_event, None)
            .await
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
                Err(reqwest_eventsource::Error::InvalidStatusCode(status, resp)) => {
                    // Server returned a non-200 HTTP status (e.g. 401, 404, 502).
                    // DON'T fall back to /messages — that would re-persist the user
                    // message and call the same failing LLM again.
                    // CADE's own server hits this path too when context building
                    // fails (e.g. stale conversation_id → 404 with a JSON body
                    // explaining why), so include the body in the surfaced error.
                    es.close();
                    let detail = describe_status_error(status, resp).await;
                    return Err(crate::Error::custom(detail));
                }
                Err(e) => {
                    // Network / transport errors (connection refused, timeout, etc.).
                    // We previously fell back to POST /messages here, but if SSE
                    // failed at the transport layer the regular POST will fail
                    // with the same error after another 30 s timeout, only making
                    // the user wait longer for the same failure.  Surface the
                    // real transport error immediately instead.
                    tracing::debug!("SSE transport error: {e:?}");
                    es.close();
                    return Err(crate::Error::custom(e.to_string()));
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
        tool_name: &str,
        output: &str,
        is_error: bool,
    ) -> Result<Vec<CadeMessage>> {
        let req = json!({
            "role": "tool",
            "tool_return": {
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
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
        tool_name: &str,
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
            tool_name,
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
        tool_name: &str,
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
                "tool_name": tool_name,
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
                    if let Ok(v) = serde_json::from_str::<Value>(data)
                        && let Some(err_msg) = v["error"].as_str()
                    {
                        es.close();
                        return Err(crate::Error::custom(err_msg));
                    }
                    if let Ok(lm) = serde_json::from_str::<CadeMessage>(data) {
                        on_event(&lm);
                        messages.push(lm);
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(reqwest_eventsource::Error::InvalidStatusCode(status, resp)) => {
                    es.close();
                    let detail = describe_status_error(status, resp).await;
                    return Err(crate::Error::custom(detail));
                }
                Err(e) => {
                    // Network / transport errors.  Same rationale as
                    // stream_message_cancellable_with_images: a failing SSE
                    // connection means the next blocking POST will also fail;
                    // surface the real error immediately rather than waiting
                    // for a second 30 s timeout.
                    tracing::debug!("SSE tool-return transport error: {e:?}");
                    es.close();
                    return Err(crate::Error::custom(e.to_string()));
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

// -- Non-200 SSE handshake errors

/// Read the response body of a failed SSE handshake and build a human-readable
/// error message.  CADE's own server returns JSON bodies such as
/// `{"detail":"conversation '…' not found for agent '…'"}` on 404 — surfacing
/// that text turns an opaque "Server returned HTTP 404" into an actionable
/// message (e.g. start a new conversation).
async fn describe_status_error(status: reqwest::StatusCode, resp: reqwest::Response) -> String {
    let body = resp.text().await.unwrap_or_default();
    format_status_error(status, &body)
}

/// Pure formatting logic of [`describe_status_error`] for testability.
fn format_status_error(status: reqwest::StatusCode, body: &str) -> String {
    let json = serde_json::from_str::<serde_json::Value>(body).ok();
    let extracted: Option<&str> = json.as_ref().and_then(|v| {
        v["detail"]
            .as_str()
            .or_else(|| v["error"]["message"].as_str())
            .or_else(|| v["error"].as_str())
    });
    let detail = extracted.unwrap_or_else(|| body.trim());
    if detail.is_empty() {
        return format!("Server returned HTTP {status}");
    }
    // Cap pathological bodies (HTML error pages, stack traces) so the TUI
    // error line stays readable.
    const MAX_CHARS: usize = 300;
    let mut detail = detail.to_string();
    if detail.chars().count() > MAX_CHARS {
        detail = detail.chars().take(MAX_CHARS).collect();
        detail.push('…');
    }
    format!("Server returned HTTP {status}: {detail}")
}

/// Pure SSE frame parser: extract the `data:` payload from raw SSE text.
/// Returns the list of data payloads in order (including `"[DONE]"`).
fn parse_sse_data(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("data: ") {
            buf.push_str(rest);
        } else if line.starts_with(':') || line.starts_with("event:") || line.starts_with("retry:")
        {
            // SSE comments and non-data fields — ignore.
        } else if line.trim().is_empty() && !buf.is_empty() {
            out.push(std::mem::take(&mut buf));
        }
        // Any other line (e.g. continuation) is also ignored per spec.
    }
    // Flush trailing data (no final blank line).
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod parse_sse_tests {
    use super::*;

    #[test]
    fn single_event() {
        assert_eq!(parse_sse_data("data: {\"seq\":1}\n\n"), vec!["{\"seq\":1}"]);
    }

    #[test]
    fn done_marker() {
        assert_eq!(
            parse_sse_data("data: {\"seq\":1}\n\ndata: [DONE]\n\n"),
            vec!["{\"seq\":1}", "[DONE]"]
        );
    }

    #[test]
    fn multiline_data() {
        assert_eq!(
            parse_sse_data("data: line1\ndata: line2\n\n"),
            vec!["line1line2"]
        );
    }

    #[test]
    fn trailing_data_no_final_blank() {
        assert_eq!(parse_sse_data("data: only\n"), vec!["only"]);
    }

    #[test]
    fn comments_and_retry_ignored() {
        assert_eq!(
            parse_sse_data(": keepalive\nretry: 5000\ndata: x\n\n"),
            vec!["x"]
        );
    }

    #[test]
    fn empty_body() {
        assert!(parse_sse_data("").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_detail_body_is_surfaced() {
        let msg = format_status_error(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"detail":"conversation 'conv-x' not found for agent 'agent-y'"}"#,
        );
        assert!(msg.contains("conversation 'conv-x' not found"));
        assert!(msg.starts_with("Server returned HTTP 404 Not Found: "));
    }

    #[test]
    fn nested_error_message_is_surfaced() {
        let msg = format_status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"invalid api key"}}"#,
        );
        assert!(msg.ends_with(": invalid api key"));
    }

    #[test]
    fn plain_text_body_is_included() {
        let msg = format_status_error(reqwest::StatusCode::BAD_GATEWAY, "upstream connect error\n");
        assert_eq!(
            msg,
            "Server returned HTTP 502 Bad Gateway: upstream connect error"
        );
    }

    #[test]
    fn empty_body_falls_back_to_status_only() {
        let msg = format_status_error(reqwest::StatusCode::NOT_FOUND, "");
        assert_eq!(msg, "Server returned HTTP 404 Not Found");
    }

    #[test]
    fn long_bodies_are_truncated() {
        let body = "x".repeat(1_000);
        let msg = format_status_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, &body);
        assert!(msg.chars().count() < 1_000);
        assert!(msg.ends_with('…'));
    }
}

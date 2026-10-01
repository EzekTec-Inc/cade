use serde_json::Value;

use cade_api_types::{AgentInfo, ChatMessage, StreamEvent};

/// Unified cross-platform Client SDK for CADE.
/// Automatically handles target arch specific async executors and transport protocols.
#[derive(Clone, Debug)]
pub struct CadeClientSdk {
    server_url: String,
    api_key: String,
}

impl CadeClientSdk {
    /// Create a new instance of the SDK client.
    pub fn new(server_url: String, api_key: String) -> Self {
        Self {
            server_url,
            api_key,
        }
    }
}

// ── Native Target Implementation ──────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native_impl {
    use super::*;
    use futures_util::StreamExt;
    use futures_util::stream::BoxStream;
    use reqwest_eventsource::{Event, EventSource};

    impl CadeClientSdk {
        /// Fetch list of all agents.
        pub async fn list_agents(&self) -> Result<Vec<AgentInfo>, crate::Error> {
            let client = reqwest::Client::new();
            let url = format!("{}/v1/agents", self.server_url);
            let res = client
                .get(&url)
                .bearer_auth(&self.api_key)
                .send()
                .await
                .map_err(|e| crate::Error::custom(format!("list_agents: {e}")))?
                .error_for_status()
                .map_err(|e| crate::Error::custom(format!("list_agents: {e}")))?;

            let body = res
                .text()
                .await
                .map_err(|e| crate::Error::custom(format!("read_body: {e}")))?;

            serde_json::from_str(&body)
                .map_err(|e| crate::Error::custom(format!("parse_agents: {e}")))
        }

        /// Fetch messages for a given agent.
        pub async fn get_messages(
            &self,
            agent_id: &str,
            conversation_id: Option<&str>,
        ) -> Result<Vec<ChatMessage>, crate::Error> {
            let client = reqwest::Client::new();
            let url = format!("{}/v1/agents/{}/messages", self.server_url, agent_id);

            let res = client
                .get(&url)
                .bearer_auth(&self.api_key)
                .query(
                    &conversation_id
                        .map(|cid| ("conversation_id", cid))
                        .into_iter()
                        .collect::<Vec<_>>(),
                )
                .send()
                .await
                .map_err(|e| crate::Error::custom(format!("get_messages: {e}")))?
                .error_for_status()
                .map_err(|e| crate::Error::custom(format!("get_messages: {e}")))?;

            let body = res
                .text()
                .await
                .map_err(|e| crate::Error::custom(format!("read_body: {e}")))?;

            cade_api_types::decode_list(&body, "messages")
                .map_err(|e| crate::Error::custom(format!("parse_messages: {e}")))
        }

        /// Stream messages via the Server-Sent Events (SSE) pipe.
        pub async fn stream_messages(
            &self,
            agent_id: &str,
            input: &str,
            conversation_id: Option<&str>,
        ) -> Result<BoxStream<'static, Result<StreamEvent, crate::Error>>, crate::Error> {
            let client = reqwest::Client::new();
            let url = format!("{}/v1/agents/{}/messages/stream", self.server_url, agent_id);
            let mut body = serde_json::json!({ "input": input });
            if let Some(cid) = conversation_id {
                body["conversation_id"] = Value::String(cid.to_string());
            }

            let request = client.post(&url).bearer_auth(&self.api_key).json(&body);

            let event_source = EventSource::new(request)
                .map_err(|e| crate::Error::custom(format!("event_source: {e}")))?;

            // EventSource normally reconnects its request. Repeating this POST
            // could start another run, so yield an error once and close instead.
            let s = futures_util::stream::unfold(
                (event_source, false),
                |(mut source, done)| async move {
                    if done {
                        return None;
                    }
                    loop {
                        match source.next().await {
                            Some(Ok(Event::Open)) => continue,
                            Some(Ok(Event::Message(message)))
                                if message.data.trim() == "[DONE]" =>
                            {
                                source.close();
                                return None;
                            }
                            Some(Ok(Event::Message(message))) => {
                                let event =
                                    serde_json::from_str::<StreamEvent>(message.data.trim())
                                        .map_err(|e| {
                                            crate::Error::custom(format!("parse_event: {e}"))
                                        });
                                let done = event.as_ref().map_or(true, StreamEvent::is_terminal);
                                if done {
                                    source.close();
                                }
                                return Some((event, (source, done)));
                            }
                            Some(Err(error)) => {
                                source.close();
                                return Some((
                                    Err(crate::Error::custom(format!("stream_err: {error}"))),
                                    (source, true),
                                ));
                            }
                            None => return None,
                        }
                    }
                },
            );

            Ok(s.boxed())
        }
    }
}

// ── WebAssembly Target Implementation ─────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use super::*;
    use futures::SinkExt;
    use futures_util::StreamExt;
    use futures_util::stream::BoxStream;
    use js_sys::Reflect;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

    impl CadeClientSdk {
        async fn api_request(
            &self,
            method: &str,
            path: &str,
            body: Option<&str>,
        ) -> Result<String, crate::Error> {
            let window = web_sys::window().ok_or_else(|| crate::Error::custom("No window"))?;
            let opts = RequestInit::new();
            opts.set_method(method);
            opts.set_mode(RequestMode::Cors);

            if let Some(body_str) = body {
                let js_body = JsValue::from_str(body_str);
                opts.set_body(&js_body);
            }

            let url = format!("{}{}", self.server_url, path);
            let request = Request::new_with_str_and_init(&url, &opts)
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            request
                .headers()
                .set("Authorization", &format!("Bearer {}", self.api_key))
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;
            request
                .headers()
                .set("Content-Type", "application/json")
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            let resp_value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;
            let resp: Response = resp_value
                .dyn_into()
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            if !resp.ok() {
                return Err(crate::Error::custom(format!(
                    "HTTP error: {}",
                    resp.status()
                )));
            }

            let text_value = JsFuture::from(
                resp.text()
                    .map_err(|e| crate::Error::custom(format!("{:?}", e)))?,
            )
            .await
            .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            Ok(text_value.as_string().unwrap_or_default())
        }

        /// Fetch list of all agents.
        pub async fn list_agents(&self) -> Result<Vec<AgentInfo>, crate::Error> {
            let body = self.api_request("GET", "/v1/agents", None).await?;
            serde_json::from_str(&body)
                .map_err(|e| crate::Error::custom(format!("JSON parse: {e}")))
        }

        /// Fetch messages for a given agent.
        pub async fn get_messages(
            &self,
            agent_id: &str,
            conversation_id: Option<&str>,
        ) -> Result<Vec<ChatMessage>, crate::Error> {
            let path = match conversation_id {
                Some(cid) => {
                    let encoded: String = cid
                        .bytes()
                        .map(|byte| {
                            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                                (byte as char).to_string()
                            } else {
                                format!("%{byte:02X}")
                            }
                        })
                        .collect();
                    format!("/v1/agents/{agent_id}/messages?conversation_id={encoded}")
                }
                None => format!("/v1/agents/{agent_id}/messages"),
            };
            let body = self.api_request("GET", &path, None).await?;
            cade_api_types::decode_list(&body, "messages")
                .map_err(|e| crate::Error::custom(format!("JSON parse: {e}")))
        }

        /// Stream messages via the Server-Sent Events (SSE) pipe.
        pub async fn stream_messages(
            &self,
            agent_id: &str,
            input: &str,
            conversation_id: Option<&str>,
        ) -> Result<BoxStream<'static, Result<StreamEvent, crate::Error>>, crate::Error> {
            let window = web_sys::window().ok_or_else(|| crate::Error::custom("No window"))?;
            let path = format!("/v1/agents/{agent_id}/messages/stream");
            let mut body_obj = serde_json::json!({ "input": input });
            if let Some(cid) = conversation_id {
                body_obj["conversation_id"] = Value::String(cid.to_string());
            }
            let body_str = body_obj.to_string();

            let opts = RequestInit::new();
            opts.set_method("POST");
            opts.set_mode(RequestMode::Cors);
            let js_body = JsValue::from_str(&body_str);
            opts.set_body(&js_body);

            let url = format!("{}{}", self.server_url, path);
            let request = Request::new_with_str_and_init(&url, &opts)
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;
            request
                .headers()
                .set("Authorization", &format!("Bearer {}", self.api_key))
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;
            request
                .headers()
                .set("Content-Type", "application/json")
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            let resp_value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;
            let resp: Response = resp_value
                .dyn_into()
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            if !resp.ok() {
                return Err(crate::Error::custom(format!(
                    "HTTP error: {}",
                    resp.status()
                )));
            }

            let stream = resp
                .body()
                .ok_or_else(|| crate::Error::custom("No response body"))?;
            let reader: ReadableStreamDefaultReader = stream
                .get_reader()
                .dyn_into()
                .map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            let (mut tx, rx) =
                futures::channel::mpsc::channel::<Result<StreamEvent, crate::Error>>(100);

            wasm_bindgen_futures::spawn_local(async move {
                let mut decoder = cade_api_types::SseDecoder::default();
                loop {
                    let result_val = JsFuture::from(reader.read()).await;
                    let result = match result_val {
                        Ok(val) => val,
                        Err(e) => {
                            let _ = tx.send(Err(crate::Error::custom(format!("{:?}", e)))).await;
                            break;
                        }
                    };

                    let done = Reflect::get(&result, &JsValue::from_str("done"))
                        .map(|v| v.as_bool().unwrap_or(false))
                        .unwrap_or(false);

                    if done {
                        for data in decoder.finish() {
                            if data.trim() != "[DONE]" {
                                let event = serde_json::from_str::<StreamEvent>(&data)
                                    .map_err(|e| crate::Error::custom(format!("parse_event: {e}")));
                                if tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                        }
                        break;
                    }

                    let value = match Reflect::get(&result, &JsValue::from_str("value")) {
                        Ok(val) => val,
                        Err(e) => {
                            let _ = tx.send(Err(crate::Error::custom(format!("{:?}", e)))).await;
                            break;
                        }
                    };

                    if value.is_null() || value.is_undefined() {
                        continue;
                    }

                    let uint8array: js_sys::Uint8Array = match value.dyn_into() {
                        Ok(arr) => arr,
                        Err(e) => {
                            let _ = tx.send(Err(crate::Error::custom(format!("{:?}", e)))).await;
                            break;
                        }
                    };
                    for data in decoder.push(&uint8array.to_vec()) {
                        if data.trim() == "[DONE]" {
                            let _ = JsFuture::from(reader.cancel()).await;
                            return;
                        }
                        let event = serde_json::from_str::<StreamEvent>(&data)
                            .map_err(|e| crate::Error::custom(format!("parse_event: {e}")));
                        let terminal = event.as_ref().is_ok_and(StreamEvent::is_terminal);
                        if tx.send(event).await.is_err() {
                            let _ = JsFuture::from(reader.cancel()).await;
                            return;
                        }
                        if terminal {
                            let _ = JsFuture::from(reader.cancel()).await;
                            return;
                        }
                    }
                }
            });

            Ok(rx.boxed())
        }
    }
}

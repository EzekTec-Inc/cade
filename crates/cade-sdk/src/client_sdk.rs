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
            let mut url = format!("{}/v1/agents/{}/messages", self.server_url, agent_id);
            if let Some(cid) = conversation_id {
                url = format!("{url}?conversation_id={cid}");
            }

            let res = client
                .get(&url)
                .bearer_auth(&self.api_key)
                .send()
                .await
                .map_err(|e| crate::Error::custom(format!("get_messages: {e}")))?;

            let body = res
                .text()
                .await
                .map_err(|e| crate::Error::custom(format!("read_body: {e}")))?;

            serde_json::from_str(&body)
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

            let s = event_source
                .map(|item| match item {
                    Ok(Event::Message(msg)) => {
                        let trimmed = msg.data.trim();
                        if trimmed == "[DONE]" {
                            Err(crate::Error::custom("DONE"))
                        } else {
                            serde_json::from_str::<StreamEvent>(trimmed)
                                .map_err(|e| crate::Error::custom(format!("parse_event: {e}")))
                        }
                    }
                    Ok(_) => Err(crate::Error::custom("Non-message event")),
                    Err(e) => Err(crate::Error::custom(format!("stream_err: {e}"))),
                })
                .filter(|r| {
                    futures_util::future::ready(
                        !matches!(r, Err(e) if e.to_string().contains("DONE")),
                    )
                });

            Ok(s.boxed())
        }
    }
}

// ── WebAssembly Target Implementation ─────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use super::*;
    use futures_util::StreamExt;
    use futures_util::stream::BoxStream;
    use js_sys::Reflect;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response, TextDecoder,
    };

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
                Some(cid) => format!("/v1/agents/{agent_id}/messages?conversation_id={cid}"),
                None => format!("/v1/agents/{agent_id}/messages"),
            };
            let body = self.api_request("GET", &path, None).await?;
            serde_json::from_str(&body)
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
            let decoder =
                TextDecoder::new().map_err(|e| crate::Error::custom(format!("{:?}", e)))?;

            let (mut tx, rx) =
                futures_util::channel::mpsc::channel::<Result<StreamEvent, crate::Error>>(100);

            wasm_bindgen_futures::spawn_local(async move {
                let mut buffer = String::new();
                loop {
                    let result_val = JsFuture::from(reader.read()).await;
                    let result = match result_val {
                        Ok(val) => val,
                        Err(e) => {
                            let _ = tx.start_send(Err(crate::Error::custom(format!("{:?}", e))));
                            break;
                        }
                    };

                    let done = Reflect::get(&result, &JsValue::from_str("done"))
                        .map(|v| v.as_bool().unwrap_or(false))
                        .unwrap_or(false);

                    if done {
                        break;
                    }

                    let value = match Reflect::get(&result, &JsValue::from_str("value")) {
                        Ok(val) => val,
                        Err(e) => {
                            let _ = tx.start_send(Err(crate::Error::custom(format!("{:?}", e))));
                            break;
                        }
                    };

                    if value.is_null() || value.is_undefined() {
                        continue;
                    }

                    let uint8array: js_sys::Uint8Array = match value.dyn_into() {
                        Ok(arr) => arr,
                        Err(e) => {
                            let _ = tx.start_send(Err(crate::Error::custom(format!("{:?}", e))));
                            break;
                        }
                    };
                    let chunk = match decoder.decode_with_buffer_source(&uint8array.into()) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = tx.start_send(Err(crate::Error::custom(format!("{:?}", e))));
                            break;
                        }
                    };

                    buffer.push_str(&chunk);

                    while let Some(pos) = buffer.find("\n\n") {
                        let event_str = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        for line in event_str.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                let trimmed = data.trim();
                                if trimmed == "[DONE]" {
                                    continue;
                                }
                                if let Ok(event) = serde_json::from_str::<StreamEvent>(trimmed) {
                                    let _ = tx.start_send(Ok(event));
                                }
                            }
                        }
                    }
                }
            });

            Ok(rx.boxed())
        }
    }
}

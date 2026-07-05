use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Clone, PartialEq)]
struct EndpointDef {
    method: Method,
    path: &'static str,
    title: &'static str,
    desc: &'static str,
    group: &'static str,
    body_template: Option<&'static str>,
}

static ENDPOINTS: &[EndpointDef] = &[
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents",
        title: "List Agents",
        desc: "Returns all configured agents on the server.",
        group: "Agents",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents/{id}/messages",
        title: "Get Messages",
        desc: "Fetch message history for an agent, with optional conversation_id filter.",
        group: "Messages",
        body_template: None,
    },
    EndpointDef {
        method: Method::Post,
        path: "/v1/agents/{id}/messages",
        title: "Send Message (Blocking)",
        desc: "Send a message to an agent and wait for the full response.",
        group: "Messages",
        body_template: Some(r#"{"input": "Hello, what do you know?"}"#),
    },
    EndpointDef {
        method: Method::Post,
        path: "/v1/agents/{id}/messages/stream",
        title: "Send Message (Streaming)",
        desc: "Send a message and receive the response as an SSE stream of events.",
        group: "Messages",
        body_template: Some(r#"{"input": "Hello, what do you know?"}"#),
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents/{id}/conversations",
        title: "List Conversations",
        desc: "List all conversations for an agent.",
        group: "Conversations",
        body_template: None,
    },
    EndpointDef {
        method: Method::Post,
        path: "/v1/agents/{id}/conversations",
        title: "Create Conversation",
        desc: "Create a new conversation for an agent.",
        group: "Conversations",
        body_template: Some(r#"{"title": "my-conversation"}"#),
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents/{id}/memory",
        title: "List Memory Blocks",
        desc: "Retrieve core memory blocks for an agent.",
        group: "Memory",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents/{id}/metrics",
        title: "Get Agent Metrics",
        desc: "Token usage and cost metrics for an agent.",
        group: "Metrics",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/agents/{id}/context_stats",
        title: "Get Context Stats",
        desc: "Context window telemetry (budget, tokens, turns).",
        group: "Metrics",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/providers",
        title: "List Providers",
        desc: "List all configured LLM providers.",
        group: "Providers",
        body_template: None,
    },
    EndpointDef {
        method: Method::Post,
        path: "/v1/providers",
        title: "Add Provider",
        desc: "Add or update an LLM provider.",
        group: "Providers",
        body_template: Some(r#"{"name": "my-provider", "kind": "openai", "api_key": "sk-..."}"#),
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/models",
        title: "List Models",
        desc: "List all available models from configured providers.",
        group: "Models",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/mcp",
        title: "List MCP Servers",
        desc: "List all MCP servers and their exposed tools.",
        group: "MCP",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/tools",
        title: "List Tools",
        desc: "List all registered tools.",
        group: "Tools",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/health",
        title: "Health Check",
        desc: "Server health check (no auth required).",
        group: "System",
        body_template: None,
    },
    EndpointDef {
        method: Method::Get,
        path: "/v1/config",
        title: "Server Config",
        desc: "Server configuration: provider, default model, version.",
        group: "System",
        body_template: None,
    },
];

#[component]
pub fn CodeView() -> Element {
    let state = use_context::<AppState>();

    let mut selected_group = use_signal(|| String::new());
    let mut selected_endpoint = use_signal(|| Option::<usize>::None);
    let mut agent_id_input = use_signal(String::new);
    let mut request_body = use_signal(String::new);
    let mut response_output = use_signal(String::new);
    let mut is_sending = use_signal(|| false);

    let groups: Vec<&str> = {
        let mut g: Vec<&str> = Vec::new();
        for ep in ENDPOINTS {
            if !g.contains(&ep.group) {
                g.push(ep.group);
            }
        }
        g
    };

    let filtered: Vec<(usize, &EndpointDef)> = if selected_group().is_empty() {
        ENDPOINTS.iter().enumerate().collect()
    } else {
        ENDPOINTS
            .iter()
            .enumerate()
            .filter(|(_, ep)| ep.group == selected_group())
            .collect()
    };

    let current_ep = selected_endpoint().and_then(|idx| ENDPOINTS.get(idx));

    let has_detail = current_ep.is_some();
    let detail_ep = current_ep;

    let detail_method_color = detail_ep
        .map(|ep| match ep.method {
            Method::Get => "text-emerald-400 bg-emerald-500/10 border-emerald-500/20",
            Method::Post => "text-blue-400 bg-blue-500/10 border-blue-500/20",
            Method::Delete => "text-red-400 bg-red-500/10 border-red-500/20",
        })
        .unwrap_or("");
    let detail_method_str = detail_ep.map(|ep| ep.method.as_str()).unwrap_or("");
    let detail_path = detail_ep.map(|ep| ep.path).unwrap_or("");
    let detail_title = detail_ep.map(|ep| ep.title).unwrap_or("");
    let detail_desc = detail_ep.map(|ep| ep.desc).unwrap_or("");
    let detail_needs_agent_id = detail_ep
        .map(|ep| ep.path.contains("{id}"))
        .unwrap_or(false);
    let detail_supports_body = detail_ep
        .map(|ep| ep.method == Method::Post || ep.method == Method::Delete)
        .unwrap_or(false);
    let method_badge_class =
        format!("text-[11px] font-bold px-2 py-0.5 rounded border {detail_method_color}");

    use_effect(move || {
        if let Some(ep) = selected_endpoint().and_then(|idx| ENDPOINTS.get(idx)) {
            if let Some(tmpl) = ep.body_template {
                request_body.set(tmpl.to_string());
            } else {
                request_body.set(String::new());
            }
        }
    });

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text flex flex-col",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218] shrink-0",
                h1 { class: "text-lg font-semibold text-white", "API Playground" }
                span { class: "text-xs text-gray-500", "Explore and test CADE API endpoints" }
            }

            div { class: "flex flex-1 overflow-hidden",
                // Left panel — endpoint list
                div { class: "w-[320px] border-r border-[#111218] flex flex-col shrink-0 overflow-hidden",
                    div { class: "p-4 border-b border-[#111218]",
                        select {
                            class: "w-full bg-[#16171d] text-gray-300 text-xs border border-[#272833] rounded-md px-3 py-2 outline-none",
                            value: "{selected_group}",
                            onchange: move |e| {
                                selected_group.set(e.value().clone());
                                selected_endpoint.set(None);
                            },
                            option { value: "", "All endpoints" }
                            for g in &groups {
                                option { value: *g, "{g}" }
                            }
                        }
                    }
                    div { class: "flex-1 overflow-y-auto p-2 space-y-1",
                        {filtered.iter().map(|(idx, ep)| {
                            let is_active = selected_endpoint() == Some(*idx);
                            let mc = match ep.method {
                                Method::Get => "text-emerald-400",
                                Method::Post => "text-blue-400",
                                Method::Delete => "text-red-400",
                            };
                            let bg = if is_active {
                                "bg-[#16171d]"
                            } else {
                                "hover:bg-[#111218]"
                            };
                            let ep_idx = *idx;
                            let ms = ep.method.as_str();
                            let t = ep.title;
                            let p = ep.path;
                            let bg_class = format!("flex items-center space-x-3 px-3 py-2.5 rounded-md cursor-pointer transition {bg}");
                            let mc_class = format!("text-[10px] font-bold {mc} w-10 shrink-0");
                            rsx! {
                                div {
                                    class: "{bg_class}",
                                    onclick: move |_| {
                                        selected_endpoint.set(Some(ep_idx));
                                    },
                                    span { class: "{mc_class}", "{ms}" }
                                    div { class: "flex flex-col min-w-0",
                                        span { class: "text-white text-xs font-medium truncate", "{t}" }
                                        span { class: "text-gray-600 text-[10px] truncate font-mono", "{p}" }
                                    }
                                }
                            }
                        })}
                    }
                }

                // Right panel
                div { class: "flex-1 flex flex-col overflow-hidden",
                    if has_detail {
                        div { class: "flex-1 overflow-y-auto p-8 space-y-6",
                            div { class: "space-y-2",
                                div { class: "flex items-center space-x-3",
                                    span { class: "{method_badge_class}",
                                        "{detail_method_str}"
                                    }
                                    span { class: "text-gray-300 font-mono text-sm", "{detail_path}" }
                                }
                                h2 { class: "text-white font-semibold text-base", "{detail_title}" }
                                p { class: "text-gray-400 text-xs", "{detail_desc}" }
                            }
                            if detail_needs_agent_id {
                                div { class: "space-y-1.5",
                                    label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "Agent ID" }
                                    input {
                                        class: "w-full bg-[#16171d] text-gray-200 text-sm border border-[#272833] rounded-md px-3 py-2 outline-none focus:border-[#ff7c5c]",
                                        placeholder: "agent-xxx...",
                                        value: "{agent_id_input}",
                                        oninput: move |e| agent_id_input.set(e.value().clone()),
                                    }
                                }
                            }
                            if detail_supports_body {
                                div { class: "space-y-1.5",
                                    label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "Request Body" }
                                    textarea {
                                        class: "w-full bg-[#16171d] text-gray-200 text-xs font-mono border border-[#272833] rounded-md px-3 py-2 outline-none focus:border-[#ff7c5c] resize-none h-28",
                                        placeholder: "{{}}",
                                        value: "{request_body}",
                                        oninput: move |e| request_body.set(e.value().clone()),
                                    }
                                }
                            }
                            button {
                                class: {
                                    if is_sending() {
                                        "bg-[#ff7c5c]/50 text-white text-xs font-semibold px-5 py-2 rounded-lg cursor-not-allowed"
                                    } else {
                                        "bg-[#ff7c5c] hover:bg-[#e26a4f] text-white text-xs font-semibold px-5 py-2 rounded-lg transition"
                                    }
                                },
                                disabled: is_sending(),
                                onclick: move |_| {
                                    if let Some(ep) = current_ep {
                                        let p = ep.path.replace("{id}", &agent_id_input());
                                        let key = (state.api_key)();
                                        let body = if detail_supports_body {
                                            let b = request_body();
                                            if b.is_empty() { None } else { Some(b) }
                                        } else {
                                            None
                                        };
                                        is_sending.set(true);
                                        response_output.set(String::new());
                                        let ms = detail_method_str.to_string();
                                        let pc = p.clone();
                                        let st = state;
                                        spawn(async move {
                                            let result = api::api_request(&ms, &pc, body.as_deref(), &key).await;
                                            match result {
                                                Ok(body) => {
                                                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                                                        if let Ok(pretty) = serde_json::to_string_pretty(&val) {
                                                            response_output.set(pretty);
                                                            return;
                                                        }
                                                    }
                                                    response_output.set(body);
                                                }
                                                Err(e) => {
                                                    response_output.set(format!("Error: {e}"));
                                                    add_toast(&st, ToastLevel::Error, "Request failed", e);
                                                }
                                            }
                                            is_sending.set(false);
                                        });
                                    }
                                },
                                if is_sending() { "Sending..." } else { "Send Request" }
                            }
                            if !response_output().is_empty() {
                                div { class: "space-y-2",
                                    div { class: "flex items-center justify-between",
                                        h3 { class: "text-xs font-semibold text-white", "Response" }
                                        button {
                                            class: "text-[10px] text-gray-500 hover:text-white transition",
                                            onclick: move |_| response_output.set(String::new()),
                                            "Clear"
                                        }
                                    }
                                    pre { class: "bg-[#0d0e12] border border-[#272833] rounded-xl p-4 text-xs font-mono text-gray-300 overflow-x-auto max-h-96 overflow-y-auto whitespace-pre-wrap",
                                        "{response_output}"
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "flex-1 flex items-center justify-center",
                            div { class: "text-center space-y-3 select-none",
                                div { class: "text-4xl text-gray-600", "\u{1f50c}" }
                                h2 { class: "text-white font-semibold text-base", "Select an endpoint" }
                                p { class: "text-gray-500 text-xs max-w-sm",
                                    "Choose an API endpoint from the left panel to send a test request."
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

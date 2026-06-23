use dioxus::prelude::*;

use crate::api;
use crate::types::{add_toast, AppState, ToastLevel};

#[component]
pub fn ToolsView() -> Element {
    let state = use_context::<AppState>();
    let servers = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);

    let key = state.api_key;
    use_effect(move || {
        let k = key;
        let st = state;
        let mut srv = servers;
        let mut busy = fetching;
        spawn(async move {
            match api::list_mcp_servers(&k()).await {
                Ok(data) => srv.set(data),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch MCP servers", e),
            }
            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = servers().clone();

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Tools" }
            }
            div { class: "p-10 space-y-4",
                h2 { class: "text-sm font-semibold text-white", "MCP Servers & Tools" }
                if fetching() {
                    for _ in 0..2 {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/4 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-1/2" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                        p { class: "text-gray-500 text-sm", "No MCP servers configured." }
                    }
                } else {
                    {items.into_iter().map(|s| {
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let tools = s.get("tools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let tool_count = tools.len();
                        rsx! {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5",
                                div { class: "flex items-center justify-between mb-3",
                                    div { class: "flex items-center space-x-3",
                                        span { class: "text-white font-semibold text-sm", "{name}" }
                                        span { class: "text-[10px] bg-[#1f212a] text-gray-400 border border-[#272833] rounded px-1.5 py-0.5", "{tool_count} tools" }
                                    }
                                }
                                if !tools.is_empty() {
                                    div { class: "flex flex-wrap gap-2",
                                        {tools.into_iter().map(|t| {
                                            let tool_name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                            let tool_desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            rsx! {
                                                div { class: "bg-[#1f212a] border border-[#272833] rounded-lg px-3 py-2 text-xs",
                                                    div { class: "text-purple-400 font-medium", "{tool_name}" }
                                                    if !tool_desc.is_empty() {
                                                        div { class: "text-gray-500 mt-0.5 line-clamp-2", "{tool_desc}" }
                                                    }
                                                }
                                            }
                                        })}
                                    }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

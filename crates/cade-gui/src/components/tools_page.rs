use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn ToolsView() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let servers = use_signal(Vec::<serde_json::Value>::new);
    let approvals = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);

    use_effect(move || {
        let st = state;
        let mut srv = servers;
        let mut apprs = approvals;
        let mut busy = fetching;
        let api_client = client();

        spawn(async move {
            match api_client.list_mcp_servers().await {
                Ok(data) => srv.set(data),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch MCP servers", e),
            }

            match api_client.list_approvals().await {
                Ok(data) => {
                    if let Some(arr) = data.get("approvals").and_then(|v| v.as_array()) {
                        apprs.set(arr.clone());
                    }
                }
                Err(e) => add_toast(
                    &st,
                    ToastLevel::Error,
                    "Failed to fetch pending approvals",
                    e,
                ),
            }

            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = servers().clone();
    let pending_approvals: Vec<serde_json::Value> = approvals().clone();

    // Handler to authorize or deny a tool execution
    let handle_action = move |id: String, action: String| {
        let api_client = client();
        let st = state;
        let mut apprs = approvals;
        spawn(async move {
            match api_client.action_approval(&id, &action).await {
                Ok(_) => {
                    let msg = if action == "approve" {
                        "Approved successfully"
                    } else {
                        "Permission denied"
                    };
                    add_toast(&st, ToastLevel::Success, msg, format!("Request ID: {}", id));
                    // Refresh the pending list
                    if let Ok(data) = api_client.list_approvals().await {
                        if let Some(arr) = data.get("approvals").and_then(|v| v.as_array()) {
                            apprs.set(arr.clone());
                        }
                    }
                }
                Err(e) => add_toast(&st, ToastLevel::Error, "Action failed", e),
            }
        });
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Tools & Approvals" }
            }

            div { class: "p-10 space-y-10",
                // ── Section 1: Headless Approvals Queue ─────────────────────────
                div { class: "space-y-4",
                    div { class: "flex items-center space-x-3",
                        h2 { class: "text-sm font-semibold text-white", "Human-in-the-Loop Approvals" }
                        if !pending_approvals.is_empty() {
                            span { class: "text-[10px] bg-red-500/10 text-red-400 border border-red-500/20 rounded-full px-2 py-0.5 font-bold animate-pulse",
                                "{pending_approvals.len()} Pending"
                            }
                        }
                    }
                    p { class: "text-xs text-gray-500 max-w-2xl leading-relaxed",
                        "When background subagents request permissions to modify files or execute shell commands, their operations are held here until authorized."
                    }

                    if pending_approvals.is_empty() {
                        div { class: "bg-[#16171d]/30 border border-[#272833]/50 rounded-xl p-6 text-center select-none",
                            p { class: "text-gray-500 text-xs", "No pending approvals. All background systems running smoothly." }
                        }
                    } else {
                        div { class: "grid grid-cols-1 gap-4",
                            {pending_approvals.into_iter().map(|a| {
                                let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                let subagent_id = a.get("subagent_id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                let tool_name = a.get("tool_name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                let arguments = a.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}").to_string();
                                let id_app = id.clone();
                                let id_deny = id.clone();
                                rsx! {
                                    div { class: "bg-[#16171d] border border-yellow-500/20 hover:border-yellow-500/40 rounded-xl p-5 transition duration-150 flex flex-col md:flex-row justify-between items-start md:items-center gap-4",
                                        div { class: "space-y-1.5 flex-1 min-w-0",
                                            div { class: "flex items-center space-x-2.5",
                                                span { class: "text-yellow-500 text-xs font-bold uppercase tracking-wider", "Pending Authorization" }
                                                span { class: "text-[10px] text-gray-500 font-mono", "ID: {id}" }
                                            }
                                            h3 { class: "text-white font-semibold text-sm",
                                                "Subagent "
                                                span { class: "text-purple-400 font-mono", "[{subagent_id}]" }
                                                " requests "
                                                span { class: "text-emerald-400 font-mono", "{tool_name}" }
                                            }
                                            pre { class: "bg-[#0f1115] text-[11px] text-gray-400 font-mono p-3 rounded-lg overflow-x-auto border border-[#272833] max-w-full",
                                                "{arguments}"
                                            }
                                        }
                                        div { class: "flex items-center space-x-2 shrink-0 self-end md:self-center select-none",
                                            button {
                                                class: "text-xs bg-[#22c55e]/10 text-[#22c55e] border border-[#22c55e]/20 rounded-md px-3.5 py-1.5 font-semibold hover:bg-[#22c55e]/20 transition",
                                                onclick: move |_| handle_action(id_app.clone(), "approve".to_string()),
                                                "Approve"
                                            }
                                            button {
                                                class: "text-xs bg-red-500/10 text-red-400 border border-red-500/20 rounded-md px-3.5 py-1.5 font-semibold hover:bg-red-500/20 transition",
                                                onclick: move |_| handle_action(id_deny.clone(), "deny".to_string()),
                                                "Deny"
                                            }
                                        }
                                    }
                                }
                            })}
                        }
                    }
                }

                // ── Section 2: MCP Servers List ─────────────────────────────────
                div { class: "space-y-4 pt-6 border-t border-[#111218]",
                    h2 { class: "text-sm font-semibold text-white", "MCP Servers & Tools" }
                    if fetching() {
                        for _ in 0..2 {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                                div { class: "h-4 bg-[#272833] rounded w-1/4 mb-3" }
                                div { class: "h-3 bg-[#272833] rounded w-1/2" }
                            }
                        }
                    } else if items.is_empty() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center select-none",
                            p { class: "text-gray-500 text-sm", "No MCP servers configured." }
                        }
                    } else {
                        {items.into_iter().map(|s| {
                            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            let tools = s.get("tools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            let tool_count = tools.len();
                            rsx! {
                                div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5",
                                    div { class: "flex items-center justify-between mb-3 select-none",
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
                                                    div { class: "bg-[#1f212a] border border-[#272833] rounded-lg px-3 py-2 text-xs flex-1 min-w-[200px] max-w-sm",
                                                        div { class: "text-purple-400 font-medium font-mono", "{tool_name}" }
                                                        if !tool_desc.is_empty() {
                                                            div { class: "text-gray-500 mt-1 line-clamp-2 leading-relaxed", "{tool_desc}" }
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
}

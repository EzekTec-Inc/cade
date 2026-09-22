use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

#[derive(Clone, Copy, PartialEq)]
enum ToolsTab {
    Approvals,
    Catalog,
    McpServers,
}

#[component]
pub fn ToolsView() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let mut active_tab = use_signal(|| ToolsTab::Approvals);
    let mut search_filter = use_signal(String::new);
    let mut category_filter = use_signal(|| "all".to_string());

    let servers = use_signal(Vec::<serde_json::Value>::new);
    let approvals = use_signal(Vec::<serde_json::Value>::new);
    let registered_tools = use_signal(Vec::<serde_json::Value>::new);
    let is_loading = use_signal(|| true);

    // Fetch all data
    let fetch_all = move |show_toast: bool| {
        let st = state;
        let api_client = client();
        let mut srv = servers;
        let mut apprs = approvals;
        let mut tools = registered_tools;
        let mut loading = is_loading;

        loading.set(true);

        spawn(async move {
            // 1. Fetch MCP servers
            if let Ok(mcp_list) = api_client.list_mcp_servers().await {
                srv.set(mcp_list);
            }

            // 2. Fetch Pending Approvals
            if let Ok(appr_data) = api_client.list_approvals().await
                && let Some(arr) = appr_data.get("approvals").and_then(|v| v.as_array())
            {
                apprs.set(arr.clone());
            }

            // 3. Fetch Registered Tools Catalog
            if let Ok(tool_list) = api_client.list_registered_tools().await {
                tools.set(tool_list);
            }

            loading.set(false);
            if show_toast {
                add_toast(
                    &st,
                    ToastLevel::Info,
                    "Tools & Approvals refreshed",
                    "Loaded latest servers, tools, and pending queues.",
                );
            }
        });
    };

    // Initial load (silent)
    use_effect(move || {
        fetch_all(false);
    });

    // Approval action handler
    let handle_action = move |id: String, action: String| {
        let api_client = client();
        let st = state;
        let mut apprs = approvals;

        // Optimistic UI update: instantly remove item from local signal
        let current_apprs = apprs();
        let filtered: Vec<serde_json::Value> = current_apprs
            .into_iter()
            .filter(|item| item.get("id").and_then(|v| v.as_str()) != Some(&id))
            .collect();
        apprs.set(filtered);

        spawn(async move {
            match api_client.action_approval(&id, &action).await {
                Ok(_) => {
                    let msg = if action == "approve" {
                        "Permission Granted"
                    } else {
                        "Execution Rejected"
                    };
                    add_toast(&st, ToastLevel::Success, msg, format!("Approval ID: {id}"));
                }
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Approval action failed", e);
                    // Re-fetch on failure
                    if let Ok(appr_data) = api_client.list_approvals().await
                        && let Some(arr) = appr_data.get("approvals").and_then(|v| v.as_array())
                    {
                        apprs.set(arr.clone());
                    }
                }
            }
        });
    };

    let pending_list = approvals();
    let mcp_list = servers();
    let tools_list = registered_tools();
    let query = search_filter().to_lowercase();
    let category = category_filter();

    // Filter tools
    let filtered_tools: Vec<serde_json::Value> = tools_list
        .into_iter()
        .filter(|t| {
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let desc = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();

            let matches_query = query.is_empty() || name.contains(&query) || desc.contains(&query);
            let matches_category = match category.as_str() {
                "native" => !name.contains("__") && !name.starts_with("tool-mesh"),
                "mcp" => name.contains("__") || name.starts_with("tool-mesh"),
                "memory" => name.contains("memory") || name == "recall" || name == "reflect",
                "plan" => name.contains("plan") || name == "finish_task",
                _ => true,
            };

            matches_query && matches_category
        })
        .collect();

    // Filter MCP servers
    let filtered_servers: Vec<serde_json::Value> = mcp_list
        .into_iter()
        .filter(|s| {
            let name = s
                .get("name")
                .or_else(|| s.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            query.is_empty() || name.contains(&query)
        })
        .collect();

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text flex flex-col",
            // Header
            header { class: "px-8 py-5 border-b border-[#1e293b]/70 bg-[#090d16] flex flex-col md:flex-row justify-between items-start md:items-center gap-4 select-none",
                div { class: "space-y-1",
                    div { class: "flex items-center space-x-3",
                        h1 { class: "text-lg font-bold text-slate-100 tracking-tight", "Tools & Approvals" }
                        if !pending_list.is_empty() {
                            span { class: "text-[11px] bg-red-500/15 text-red-400 border border-red-500/30 rounded-full px-2.5 py-0.5 font-bold animate-pulse",
                                "{pending_list.len()} Action Required"
                            }
                        }
                    }
                    p { class: "text-xs text-slate-400",
                        "Authorize sensitive subagent actions, explore registered tool capabilities, and inspect active MCP servers."
                    }
                }

                div { class: "flex items-center space-x-3",
                    button {
                        class: "text-xs bg-[#16171d] hover:bg-[#1f212a] text-slate-300 border border-[#1e293b] rounded-lg px-3 py-1.5 font-medium transition flex items-center space-x-1.5",
                        onclick: move |_| fetch_all(true),
                        span { "↻" }
                        span { "Refresh" }
                    }
                }
            }

            // Segmented Navigation Bar
            div { class: "px-8 py-3 bg-[#070b14] border-b border-[#1e293b]/50 flex flex-wrap items-center justify-between gap-4 select-none",
                div { class: "flex items-center space-x-2",
                    button {
                        class: if active_tab() == ToolsTab::Approvals {
                            "text-xs px-3.5 py-1.5 rounded-lg bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-semibold flex items-center space-x-2 transition"
                        } else {
                            "text-xs px-3.5 py-1.5 rounded-lg text-slate-400 hover:text-slate-200 hover:bg-[#141720] border border-transparent font-medium flex items-center space-x-2 transition"
                        },
                        onclick: move |_| active_tab.set(ToolsTab::Approvals),
                        span { "🛡️" }
                        span { "Security Approvals" }
                        if !pending_list.is_empty() {
                            span { class: "text-[10px] bg-red-500/20 text-red-400 rounded-full px-1.5 py-0.2 font-bold",
                                "{pending_list.len()}"
                            }
                        }
                    }

                    button {
                        class: if active_tab() == ToolsTab::Catalog {
                            "text-xs px-3.5 py-1.5 rounded-lg bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 font-semibold flex items-center space-x-2 transition"
                        } else {
                            "text-xs px-3.5 py-1.5 rounded-lg text-slate-400 hover:text-slate-200 hover:bg-[#141720] border border-transparent font-medium flex items-center space-x-2 transition"
                        },
                        onclick: move |_| active_tab.set(ToolsTab::Catalog),
                        span { "⚡" }
                        span { "Tool Catalog" }
                        span { class: "text-[10px] bg-slate-800 text-slate-400 rounded-full px-1.5 py-0.2",
                            "{filtered_tools.len()}"
                        }
                    }

                    button {
                        class: if active_tab() == ToolsTab::McpServers {
                            "text-xs px-3.5 py-1.5 rounded-lg bg-purple-500/15 text-purple-400 border border-purple-500/30 font-semibold flex items-center space-x-2 transition"
                        } else {
                            "text-xs px-3.5 py-1.5 rounded-lg text-slate-400 hover:text-slate-200 hover:bg-[#141720] border border-transparent font-medium flex items-center space-x-2 transition"
                        },
                        onclick: move |_| active_tab.set(ToolsTab::McpServers),
                        span { "🔌" }
                        span { "MCP Gateway" }
                        span { class: "text-[10px] bg-slate-800 text-slate-400 rounded-full px-1.5 py-0.2",
                            "{filtered_servers.len()}"
                        }
                    }
                }

                // Filter search
                div { class: "flex items-center space-x-2",
                    input {
                        class: "bg-[#141720] text-slate-200 text-xs rounded-lg px-3 py-1.5 outline-none border border-[#1e293b] w-64 placeholder-slate-500 focus:border-indigo-500/60 transition",
                        placeholder: "Filter tools or servers...",
                        value: "{search_filter}",
                        oninput: move |e| search_filter.set(e.value().clone()),
                    }
                }
            }

            // Content Area
            div { class: "p-8 flex-1 space-y-6",
                match active_tab() {
                    ToolsTab::Approvals => rsx! {
                        div { class: "space-y-4",
                            if is_loading() {
                                div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center",
                                    p { class: "text-slate-400 text-xs animate-pulse", "Checking pending approval queue..." }
                                }
                            } else if pending_list.is_empty() {
                                div { class: "bg-[#090d16]/40 border border-[#1e293b]/60 rounded-xl p-10 text-center select-none space-y-2",
                                    div { class: "text-2xl", "✓" }
                                    h3 { class: "text-sm font-semibold text-slate-200", "No Pending Approvals" }
                                    p { class: "text-xs text-slate-500 max-w-md mx-auto leading-relaxed",
                                        "All background subagents and tool executions are running within autonomous permissions. Sensitive requests will appear here for review."
                                    }
                                }
                            } else {
                                div { class: "grid grid-cols-1 gap-4",
                                    {pending_list.into_iter().map(|a| {
                                        let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let subagent_id = a.get("subagent_id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let tool_name = a.get("tool_name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let arguments = a.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}").to_string();
                                        let id_app = id.clone();
                                        let id_deny = id.clone();

                                        let is_dangerous = tool_name.contains("bash") || tool_name.contains("shell") || tool_name.contains("delete");

                                        rsx! {
                                            div {
                                                key: "{id}",
                                                class: if is_dangerous {
                                                    "bg-[#090d16] border border-red-500/30 hover:border-red-500/50 rounded-xl p-5 transition flex flex-col md:flex-row justify-between items-start md:items-center gap-5"
                                                } else {
                                                    "bg-[#090d16] border border-yellow-500/25 hover:border-yellow-500/40 rounded-xl p-5 transition flex flex-col md:flex-row justify-between items-start md:items-center gap-5"
                                                },
                                                div { class: "space-y-2 flex-1 min-w-0",
                                                    div { class: "flex items-center space-x-2.5",
                                                        if is_dangerous {
                                                            span { class: "text-red-400 bg-red-500/10 border border-red-500/20 text-[10px] font-bold px-2 py-0.5 rounded uppercase tracking-wider", "High-Impact Operation" }
                                                        } else {
                                                            span { class: "text-yellow-400 bg-yellow-500/10 border border-yellow-500/20 text-[10px] font-bold px-2 py-0.5 rounded uppercase tracking-wider", "Pending Authorization" }
                                                        }
                                                        span { class: "text-[10px] text-slate-500 font-mono", "ID: {id}" }
                                                    }
                                                    h3 { class: "text-slate-100 font-semibold text-sm flex flex-wrap items-center gap-1.5",
                                                        span { "Subagent" }
                                                        span { class: "text-purple-400 font-mono bg-[#141720] px-1.5 py-0.5 rounded text-xs border border-[#1e293b]", "{subagent_id}" }
                                                        span { "requests" }
                                                        span { class: "text-emerald-400 font-mono font-bold bg-[#141720] px-1.5 py-0.5 rounded text-xs border border-[#1e293b]", "{tool_name}" }
                                                    }
                                                    div { class: "space-y-1",
                                                        span { class: "text-[10px] text-slate-500 font-bold uppercase tracking-wider", "Payload Arguments" }
                                                        pre { class: "bg-[#040711] text-[11px] text-slate-300 font-mono p-3 rounded-lg overflow-x-auto border border-[#1e293b] max-w-full leading-relaxed",
                                                            "{arguments}"
                                                        }
                                                    }
                                                }

                                                div { class: "flex items-center space-x-2.5 shrink-0 self-end md:self-center select-none",
                                                    button {
                                                        class: "text-xs bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-400 border border-emerald-500/30 rounded-lg px-4 py-2 font-semibold transition flex items-center space-x-1.5 shadow-sm",
                                                        onclick: move |_| handle_action(id_app.clone(), "approve".to_string()),
                                                        span { "✓" }
                                                        span { "Approve" }
                                                    }
                                                    button {
                                                        class: "text-xs bg-red-500/15 hover:bg-red-500/25 text-red-400 border border-red-500/30 rounded-lg px-4 py-2 font-semibold transition flex items-center space-x-1.5 shadow-sm",
                                                        onclick: move |_| handle_action(id_deny.clone(), "deny".to_string()),
                                                        span { "✕" }
                                                        span { "Deny" }
                                                    }
                                                }
                                            }
                                        }
                                    })}
                                }
                            }
                        }
                    },

                    ToolsTab::Catalog => rsx! {
                        div { class: "space-y-4",
                            // Sub-category filters
                            div { class: "flex flex-wrap items-center gap-2 select-none",
                                for (cat_key, cat_label) in [("all", "All Tools"), ("native", "Native Core"), ("mcp", "MCP Mesh"), ("memory", "Memory & Context"), ("plan", "Planning & Tasks")] {
                                    button {
                                        key: "{cat_key}",
                                        class: if category == cat_key {
                                            "text-xs px-3 py-1 rounded-full bg-indigo-500/20 text-indigo-300 border border-indigo-500/40 font-semibold transition"
                                        } else {
                                            "text-xs px-3 py-1 rounded-full text-slate-400 hover:text-slate-200 hover:bg-[#141720] border border-[#1e293b] transition"
                                        },
                                        onclick: move |_| category_filter.set(cat_key.to_string()),
                                        "{cat_label}"
                                    }
                                }
                            }

                            if is_loading() {
                                div { class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4",
                                    for _ in 0..6 {
                                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-4 animate-pulse space-y-2",
                                            div { class: "h-4 bg-[#1e293b] rounded w-1/3" }
                                            div { class: "h-3 bg-[#1e293b] rounded w-full" }
                                            div { class: "h-3 bg-[#1e293b] rounded w-2/3" }
                                        }
                                    }
                                }
                            } else if filtered_tools.is_empty() {
                                div { class: "bg-[#090d16]/40 border border-[#1e293b]/60 rounded-xl p-10 text-center select-none",
                                    p { class: "text-slate-400 text-sm", "No registered tools matched the selected filter." }
                                }
                            } else {
                                div { class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4",
                                    {filtered_tools.into_iter().map(|t| {
                                        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("No description provided.").to_string();
                                        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

                                        let badge_type = if name.contains("__") {
                                            ("MCP", "text-purple-400 bg-purple-500/10 border-purple-500/20")
                                        } else if name.contains("memory") || name == "recall" || name == "reflect" {
                                            ("MEMORY", "text-cyan-400 bg-cyan-500/10 border-cyan-500/20")
                                        } else if name.contains("plan") || name == "finish_task" {
                                            ("PLAN", "text-amber-400 bg-amber-500/10 border-amber-500/20")
                                        } else {
                                            ("CORE", "text-emerald-400 bg-emerald-500/10 border-emerald-500/20")
                                        };

                                        rsx! {
                                            div {
                                                key: "{name}",
                                                class: "bg-[#090d16] border border-[#1e293b] hover:border-slate-600 rounded-xl p-4 transition duration-150 flex flex-col justify-between space-y-2.5",
                                                div { class: "space-y-1.5",
                                                    div { class: "flex items-center justify-between gap-2",
                                                        span { class: "text-slate-100 font-mono font-bold text-xs truncate", "{name}" }
                                                        span { class: "text-[9px] font-bold px-1.5 py-0.5 rounded border {badge_type.1}", "{badge_type.0}" }
                                                    }
                                                    p { class: "text-xs text-slate-400 leading-relaxed line-clamp-3", "{desc}" }
                                                }
                                                if !id.is_empty() {
                                                    div { class: "pt-1 border-t border-[#1e293b]/40 flex justify-between items-center text-[10px] text-slate-500 font-mono",
                                                        span { "{id}" }
                                                    }
                                                }
                                            }
                                        }
                                    })}
                                }
                            }
                        }
                    },

                    ToolsTab::McpServers => rsx! {
                        div { class: "space-y-4",
                            if is_loading() {
                                div { class: "space-y-3",
                                    for _ in 0..3 {
                                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 animate-pulse space-y-2",
                                            div { class: "h-4 bg-[#1e293b] rounded w-1/4" }
                                            div { class: "h-3 bg-[#1e293b] rounded w-1/2" }
                                        }
                                    }
                                }
                            } else if filtered_servers.is_empty() {
                                div { class: "bg-[#090d16]/40 border border-[#1e293b]/60 rounded-xl p-10 text-center select-none",
                                    p { class: "text-slate-400 text-sm", "No MCP servers configured or registered." }
                                }
                            } else {
                                div { class: "space-y-4",
                                    {filtered_servers.into_iter().map(|s| {
                                        let name = s.get("name").or_else(|| s.get("key")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let command = s.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let status_val = s.get("status").and_then(|v| v.as_str()).unwrap_or("ready").to_string();
                                        let error_val = s.get("error").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let disabled = s.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);

                                        let tools = s.get("tools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                        let tool_count = tools.len();

                                        let (status_text, status_class) = if disabled {
                                            ("Disabled", "text-slate-400 bg-slate-800 border-slate-700")
                                        } else if status_val == "failed" {
                                            ("Failed", "text-red-400 bg-red-500/10 border-red-500/20")
                                        } else if status_val == "timeout" {
                                            ("Timeout", "text-amber-400 bg-amber-500/10 border-amber-500/20")
                                        } else {
                                            ("Ready", "text-emerald-400 bg-emerald-500/10 border-emerald-500/20")
                                        };

                                        rsx! {
                                            div {
                                                key: "{name}",
                                                class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 space-y-3.5",
                                                div { class: "flex flex-col sm:flex-row justify-between items-start sm:items-center gap-2 select-none",
                                                    div { class: "flex items-center space-x-3",
                                                        span { class: "text-slate-100 font-bold text-sm", "{name}" }
                                                        span { class: "text-[10px] font-semibold px-2 py-0.5 rounded border {status_class}", "{status_text}" }
                                                        span { class: "text-[10px] bg-[#141720] text-slate-400 border border-[#1e293b] rounded px-2 py-0.5 font-mono", "{tool_count} tools" }
                                                    }
                                                    if !command.is_empty() {
                                                        span { class: "text-[11px] text-slate-400 font-mono bg-[#040711] px-2 py-1 rounded border border-[#1e293b] truncate max-w-sm", "{command}" }
                                                    }
                                                }

                                                if !error_val.is_empty() {
                                                    div { class: "text-xs text-red-400 bg-red-500/10 border border-red-500/20 p-2.5 rounded-lg font-mono",
                                                        "Diagnostic Error: {error_val}"
                                                    }
                                                }

                                                if !tools.is_empty() {
                                                    div { class: "flex flex-wrap gap-2 pt-1",
                                                        {tools.into_iter().map(|t| {
                                                            let t_name = match t {
                                                                serde_json::Value::String(s) => s,
                                                                serde_json::Value::Object(ref map) => {
                                                                    map.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string()
                                                                }
                                                                _ => "?".to_string(),
                                                            };
                                                            rsx! {
                                                                div {
                                                                    key: "{t_name}",
                                                                    class: "bg-[#141720] border border-[#1e293b] rounded-md px-2.5 py-1 text-[11px] font-mono text-purple-300 flex items-center space-x-1.5",
                                                                    span { "⚡" }
                                                                    span { "{t_name}" }
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
                    },
                }
            }
        }
    }
}

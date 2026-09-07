use dioxus::prelude::*;
use crate::types::{AppState, SelectedPage, ToastLevel, add_toast};

/// Live Agent-Activity & Approvals Dashboard view.
#[component]
pub fn LiveView() -> Element {
    let state = use_context::<AppState>();
    let mut runs = use_signal(Vec::<serde_json::Value>::new);
    let mut is_fetching = use_signal(|| false);

    // Fetch runs whenever selected agent changes
    use_effect(move || {
        let key = (state.api_key)();
        if let Some(agent) = (state.selected_agent)() {
            let aid = agent.id.clone();
            spawn(async move {
                is_fetching.set(true);
                if let Ok(r) = crate::api::list_agent_runs(&aid, &key).await {
                    runs.set(r);
                }
                is_fetching.set(false);
            });
        }
    });

    let current_agent = (state.selected_agent)();
    let current_agent_name = current_agent
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "No Agent Selected".to_string());
    let current_agent_model = current_agent
        .as_ref()
        .and_then(|a| a.model.clone())
        .unwrap_or_else(|| "—".to_string());
    let pending_approvals = (state.pending_approvals)();

    rsx! {
        div { class: "flex-1 flex flex-col bg-[#040711] overflow-y-auto",
            // Header Bar
            header { class: "px-10 py-5 border-b border-[#1e293b]/70 bg-[#090d16]/90 backdrop-blur-md sticky top-0 z-10 flex items-center justify-between select-none",
                div { class: "flex items-center space-x-3",
                    div { class: "w-2.5 h-2.5 rounded-full bg-emerald-400 animate-pulse" }
                    h1 { class: "text-lg font-bold text-slate-100 tracking-tight flex items-center space-x-2",
                        span { "Live Activity & Runtime Telemetry" }
                    }
                }
                div { class: "flex items-center space-x-4",
                    span { class: "text-xs font-mono text-slate-400", "Agent: " span { class: "text-slate-200 font-bold", "{current_agent_name}" } }
                    span { class: "text-xs font-mono px-2.5 py-1 rounded bg-[#16171d] border border-slate-800 text-cyan-400 font-medium", "{current_agent_model}" }
                }
            }

            // Main Content Area
            div { class: "px-10 py-8 space-y-8 flex-1",

                // 1. Pending Approvals Section (if any)
                if !pending_approvals.is_empty() {
                    div { class: "rounded-xl border border-amber-500/40 bg-amber-950/20 p-6 space-y-4 shadow-lg",
                        div { class: "flex items-center justify-between",
                            div { class: "flex items-center space-x-2",
                                span { class: "text-amber-400 font-bold text-sm uppercase tracking-wider", "⚠️ Action Required — Pending Tool Approvals" }
                                span { class: "px-2 py-0.5 rounded-full text-xs font-mono bg-amber-500/20 text-amber-300 font-bold", "{pending_approvals.len()}" }
                            }
                        }
                        div { class: "space-y-3",
                            for app in pending_approvals.iter() {
                                {
                                    let app_id = app["id"].as_str().unwrap_or("").to_string();
                                    let tool_name = app["tool_name"].as_str().unwrap_or("unknown").to_string();
                                    let agent_id = app["agent_id"].as_str().unwrap_or("").to_string();
                                    let args_str = app["arguments"].to_string();
                                    let app_id_c1 = app_id.clone();
                                    let app_id_c2 = app_id.clone();
                                    let key_c1 = (state.api_key)();
                                    let key_c2 = (state.api_key)();

                                    rsx! {
                                        div { key: "{app_id}", class: "p-4 rounded-lg bg-[#0f1115] border border-amber-500/30 flex flex-col md:flex-row md:items-center justify-between gap-4",
                                            div { class: "space-y-1",
                                                div { class: "flex items-center space-x-2",
                                                    span { class: "text-xs font-mono px-2 py-0.5 rounded bg-slate-800 text-slate-300", "{agent_id}" }
                                                    span { class: "text-sm font-bold text-amber-300 font-mono", "{tool_name}" }
                                                }
                                                div { class: "text-xs font-mono text-slate-400 truncate max-w-xl", "{args_str}" }
                                            }
                                            div { class: "flex items-center space-x-3 shrink-0",
                                                button {
                                                    class: "px-4 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-bold transition-colors cursor-pointer shadow-md",
                                                    onclick: move |_| {
                                                        let id = app_id_c1.clone();
                                                        let k = key_c1.clone();
                                                        let st = state;
                                                        spawn(async move {
                                                            let client = crate::api::CadeApiClient::new(k);
                                                            match client.action_approval(&id, "approve").await {
                                                                Ok(_) => add_toast(&st, ToastLevel::Success, "Approved", format!("Approval {id} granted")),
                                                                Err(e) => add_toast(&st, ToastLevel::Error, "Failed", e),
                                                            }
                                                        });
                                                    },
                                                    "Approve"
                                                }
                                                button {
                                                    class: "px-4 py-1.5 rounded-lg bg-rose-700 hover:bg-rose-600 text-white text-xs font-bold transition-colors cursor-pointer shadow-md",
                                                    onclick: move |_| {
                                                        let id = app_id_c2.clone();
                                                        let k = key_c2.clone();
                                                        let st = state;
                                                        spawn(async move {
                                                            let client = crate::api::CadeApiClient::new(k);
                                                            match client.action_approval(&id, "deny").await {
                                                                Ok(_) => add_toast(&st, ToastLevel::Warning, "Denied", format!("Approval {id} rejected")),
                                                                Err(e) => add_toast(&st, ToastLevel::Error, "Failed", e),
                                                            }
                                                        });
                                                    },
                                                    "Deny"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 2. Active Run & Telemetry Row
                div { class: "grid grid-cols-1 md:grid-cols-3 gap-6",
                    div { class: "p-5 rounded-xl border border-slate-800 bg-[#0c101d] space-y-2",
                        div { class: "text-xs font-mono text-slate-400 uppercase", "Active Session Status" }
                        div { class: "text-xl font-extrabold text-slate-100 flex items-center space-x-2",
                            if runs().iter().any(|r| r["status"].as_str() == Some("running")) {
                                span { class: "text-emerald-400", "● Executing Turn" }
                            } else {
                                span { class: "text-slate-400", "○ Idle" }
                            }
                        }
                    }
                    div { class: "p-5 rounded-xl border border-slate-800 bg-[#0c101d] space-y-2",
                        div { class: "text-xs font-mono text-slate-400 uppercase", "Total Recorded Runs" }
                        div { class: "text-xl font-extrabold text-cyan-400 font-mono", "{runs().len()}" }
                    }
                    div { class: "p-5 rounded-xl border border-slate-800 bg-[#0c101d] space-y-2",
                        div { class: "text-xs font-mono text-slate-400 uppercase", "Pending Approvals" }
                        div { class: "text-xl font-extrabold text-amber-400 font-mono", "{pending_approvals.len()}" }
                    }
                }

                // 3. Runs History Table
                div { class: "rounded-xl border border-slate-800 bg-[#0c101d] overflow-hidden",
                    div { class: "px-6 py-4 border-b border-slate-800 flex items-center justify-between",
                        h2 { class: "text-sm font-bold text-slate-200 uppercase tracking-wider", "Recent Agent Execution Runs" }
                        if is_fetching() {
                            span { class: "text-xs text-cyan-400 font-mono animate-pulse", "Syncing runs..." }
                        }
                    }
                    if runs().is_empty() {
                        div { class: "p-12 text-center text-sm text-slate-500 font-mono", "No recorded runs for this agent." }
                    } else {
                        div { class: "overflow-x-auto",
                            table { class: "w-full text-left text-xs text-slate-300 font-mono",
                                thead { class: "bg-[#111625] text-slate-400 uppercase border-b border-slate-800",
                                    tr {
                                        th { class: "px-6 py-3", "Run ID" }
                                        th { class: "px-6 py-3", "Status" }
                                        th { class: "px-6 py-3", "Conversation" }
                                        th { class: "px-6 py-3", "Timestamp" }
                                        th { class: "px-6 py-3 text-right", "Action" }
                                    }
                                }
                                tbody { class: "divide-y divide-slate-800/60",
                                    for r in runs().iter() {
                                        {
                                            let rid = r["id"].as_str().unwrap_or("").to_string();
                                            let status = r["status"].as_str().unwrap_or("unknown").to_string();
                                            let conv = r["conversation_id"].as_str().unwrap_or("none").to_string();
                                            let created = r["created_at"].as_i64().unwrap_or(0);
                                            let is_running = status == "running";
                                            let status_color = match status.as_str() {
                                                "running" => "text-emerald-400 bg-emerald-950/60 border-emerald-800",
                                                "completed" | "done" => "text-cyan-400 bg-cyan-950/60 border-cyan-800",
                                                _ => "text-rose-400 bg-rose-950/60 border-rose-800",
                                            };

                                            rsx! {
                                                tr { key: "{rid}", class: "hover:bg-[#111625]/50 transition-colors",
                                                    td { class: "px-6 py-4 font-bold text-slate-200", "{rid}" }
                                                    td { class: "px-6 py-4",
                                                        span { class: "px-2.5 py-1 rounded-full text-[11px] font-bold border {status_color} inline-flex items-center space-x-1.5",
                                                            if is_running {
                                                                span { class: "w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" }
                                                            }
                                                            span { "{status}" }
                                                        }
                                                    }
                                                    td { class: "px-6 py-4 text-slate-400 truncate max-w-xs", "{conv}" }
                                                    td { class: "px-6 py-4 text-slate-400", "{created}" }
                                                    td { class: "px-6 py-4 text-right",
                                                        button {
                                                            class: "px-3 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-semibold cursor-pointer transition-colors",
                                                            onclick: move |_| {
                                                                let mut page = state.active_page;
                                                                page.set(SelectedPage::Chat);
                                                            },
                                                            "View Chat"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

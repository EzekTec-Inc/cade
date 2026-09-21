use crate::types::{AppState, SelectedPage, ToastLevel, add_toast};
use dioxus::prelude::*;

/// Live Agent-Activity & Approvals Dashboard view.
#[component]
pub fn LiveView() -> Element {
    let state = use_context::<AppState>();
    let runs = state.runs;
    let mut is_fetching = use_signal(|| false);

    // Continuously sync runs while Live view is active, and refresh immediately when selected agent changes
    use_effect(move || {
        let key = (state.api_key)();
        let mut runs_sig = state.runs;
        if let Some(agent) = (state.selected_agent)() {
            let aid = agent.id.clone();
            spawn(async move {
                is_fetching.set(true);
                if let Ok(r) = crate::api::list_agent_runs(&aid, &key).await {
                    runs_sig.set(r);
                }
                is_fetching.set(false);

                // Polling heartbeat (2s) to ensure telemetry never desyncs during long turns
                loop {
                    gloo_timers::future::TimeoutFuture::new(2000).await;
                    if let Ok(r) = crate::api::list_agent_runs(&aid, &key).await {
                        runs_sig.set(r);
                    }
                }
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

    let mut selected_run = use_signal(|| None::<serde_json::Value>);
    let mut is_drawer_open = use_signal(|| false);
    let mut steer_input = use_signal(String::new);
    let mut model_input = use_signal(|| "gemini/gemini-2.0-flash".to_string());
    let drawer_logs = use_signal(Vec::<String>::new);
    let is_streaming_drawer = use_signal(|| false);

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

                // 2. Active Swarm & Subagent Workflows
                div { class: "space-y-4",
                    div { class: "flex items-center justify-between",
                        div { class: "flex items-center space-x-2.5",
                            h2 { class: "text-sm font-bold text-slate-200 uppercase tracking-wider", "Active Swarm & Subagent Workflows" }
                            span { class: "text-[10px] font-mono px-2 py-0.5 rounded bg-cyan-950/60 border border-cyan-800/80 text-cyan-300 font-bold", "Real-Time Telemetry" }
                        }
                    }

                    {
                        let active_runs: Vec<_> = runs().into_iter().filter(|r| r["status"].as_str() == Some("running")).collect();
                        if !active_runs.is_empty() {
                            rsx! {
                                div { class: "grid grid-cols-1 lg:grid-cols-2 gap-4",
                                    for r in active_runs {
                                        {
                                            let r_id = r["id"].as_str().unwrap_or("").to_string();
                                            let a_id = r["agent_id"].as_str().unwrap_or("").to_string();
                                            let r_clone = r.clone();
                                            let r_id_inspect = r_id.clone();
                                            let r_id_steer = r_id.clone();
                                            let r_id_swap = r_id.clone();
                                            let r_id_cancel = r_id.clone();
                                            let key = (state.api_key)();
                                            let st = state;
                                            let mut inline_steer = use_signal(String::new);

                                            rsx! {
                                                div { key: "{r_id}", class: "p-5 rounded-xl border border-cyan-500/40 bg-[#090e1a] shadow-[0_4px_20px_rgba(6,182,212,0.12)] flex flex-col justify-between space-y-4 font-mono text-xs",
                                                    // Card Header
                                                    div { class: "flex items-center justify-between border-b border-slate-800 pb-3",
                                                        div { class: "flex items-center space-x-2",
                                                            span { class: "w-2 h-2 rounded-full bg-emerald-400 animate-ping" }
                                                            span { class: "text-xs font-bold text-slate-100 uppercase", "{r_id}" }
                                                            span { class: "text-slate-500", "·" }
                                                            span { class: "text-cyan-400 font-semibold", "{a_id}" }
                                                        }
                                                        span { class: "px-2.5 py-0.5 rounded-full text-[10px] font-bold bg-emerald-950/80 border border-emerald-600 text-emerald-300", "● RUNNING" }
                                                    }

                                                    // Inline Steer Input
                                                    div { class: "space-y-2.5",
                                                        div { class: "flex items-center space-x-2",
                                                            input {
                                                                class: "flex-1 px-3 py-1.5 rounded-lg bg-[#121826] border border-slate-700 text-slate-100 placeholder-slate-500 text-xs focus:outline-none focus:border-cyan-500",
                                                                placeholder: "Quick steer instruction...",
                                                                value: "{inline_steer}",
                                                                oninput: move |e| inline_steer.set(e.value()),
                                                            }
                                                            button {
                                                                class: "px-3 py-1.5 rounded-lg bg-cyan-600 hover:bg-cyan-500 text-white font-bold text-xs cursor-pointer transition-colors shadow",
                                                                onclick: {
                                                                    let id = r_id_steer.clone();
                                                                    let k = key.clone();
                                                                    let st_c = st;
                                                                    move |_| {
                                                                        let msg = (inline_steer)().clone();
                                                                        let mut sig = inline_steer;
                                                                        let k_inner = k.clone();
                                                                        let id_inner = id.clone();
                                                                        spawn(async move {
                                                                            let client = crate::api::CadeApiClient::new(k_inner);
                                                                            match client.steer_subagent(&id_inner, &msg).await {
                                                                                Ok(_) => {
                                                                                    add_toast(&st_c, ToastLevel::Success, "Steered", format!("Guidance sent to {id_inner}"));
                                                                                    sig.set(String::new());
                                                                                }
                                                                                Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                                            }
                                                                        });
                                                                    }
                                                                },
                                                                "Steer"
                                                            }
                                                        }

                                                        // Inline Model Dropdown & Action Buttons
                                                        div { class: "flex items-center justify-between pt-1 gap-2",
                                                            div { class: "flex items-center space-x-1.5 flex-1 min-w-0",
                                                                span { class: "text-[10px] text-slate-500 uppercase", "Model:" }
                                                                select {
                                                                    class: "px-2 py-1 rounded bg-[#121826] border border-slate-700 text-amber-300 text-xs font-mono focus:outline-none focus:border-amber-500 flex-1 truncate cursor-pointer",
                                                                    onchange: {
                                                                        let id = r_id_swap.clone();
                                                                        let k = key.clone();
                                                                        let st_c = st;
                                                                        move |e: Event<FormData>| {
                                                                            let m = e.value();
                                                                            let id_inner = id.clone();
                                                                            let k_inner = k.clone();
                                                                            spawn(async move {
                                                                                let client = crate::api::CadeApiClient::new(k_inner);
                                                                                match client.swap_subagent_model(&id_inner, &m).await {
                                                                                    Ok(_) => add_toast(&st_c, ToastLevel::Success, "Model Swapped", format!("Swapped {id_inner} to {m}")),
                                                                                    Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                                                }
                                                                            });
                                                                        }
                                                                    },
                                                                    option { value: "gemini/gemini-2.0-flash", selected: true, "gemini-2.0-flash (Fast)" }
                                                                    option { value: "anthropic/claude-haiku-4-5", "claude-haiku-4-5 (Fast)" }
                                                                    option { value: "openai/o4-mini", "o4-mini (Fast)" }
                                                                    option { value: "anthropic/claude-3-7-sonnet", "claude-3-7-sonnet (Deep)" }
                                                                    option { value: "gemini/gemini-2.5-pro", "gemini-2.5-pro (Deep)" }
                                                                }
                                                            }

                                                            div { class: "flex items-center space-x-2 shrink-0",
                                                                button {
                                                                    class: "px-3 py-1 rounded bg-cyan-900/80 hover:bg-cyan-700 text-cyan-100 border border-cyan-700/60 font-semibold cursor-pointer text-xs transition-colors",
                                                                    onclick: {
                                                                        let r_inner = r_clone.clone();
                                                                        let r_id_in = r_id_inspect.clone();
                                                                        let k_in = key.clone();
                                                                        move |_| {
                                                                            selected_run.set(Some(r_inner.clone()));
                                                                            is_drawer_open.set(true);
                                                                            let mut logs = drawer_logs;
                                                                            let mut is_str = is_streaming_drawer;
                                                                            logs.set(vec!["Following live run stream...".to_string()]);
                                                                            let r_sub = r_id_in.clone();
                                                                            let k_sub = k_in.clone();
                                                                            spawn(async move {
                                                                                is_str.set(true);
                                                                                let _ = crate::api::stream_run(&k_sub, &r_sub, None, move |evt| {
                                                                                    let mut list = logs();
                                                                                    if let Some(c) = evt.content() {
                                                                                        list.push(format!("[prose] {c}"));
                                                                                    } else if let Some(t) = evt.tool_name() {
                                                                                        list.push(format!("[tool] {t}"));
                                                                                    } else if let Some(reasoning) = evt.reasoning() {
                                                                                        list.push(format!("[thought] {reasoning}"));
                                                                                    }
                                                                                    if list.len() > 200 {
                                                                                        list.remove(0);
                                                                                    }
                                                                                    logs.set(list);
                                                                                }).await;
                                                                                is_str.set(false);
                                                                            });
                                                                        }
                                                                    },
                                                                    "🔍 Stream"
                                                                }
                                                                button {
                                                                    class: "px-3 py-1 rounded bg-rose-950/80 hover:bg-rose-800 text-rose-200 border border-rose-800/80 font-semibold cursor-pointer text-xs transition-colors",
                                                                    onclick: {
                                                                        let id = r_id_cancel.clone();
                                                                        let k = key.clone();
                                                                        let st_c = st;
                                                                        move |_| {
                                                                            let id_sub = id.clone();
                                                                            let k_sub = k.clone();
                                                                            spawn(async move {
                                                                                let client = crate::api::CadeApiClient::new(k_sub);
                                                                                match client.cancel_run(&id_sub).await {
                                                                                    Ok(_) => add_toast(&st_c, ToastLevel::Warning, "Cancelled", format!("Run {id_sub} cancelled")),
                                                                                    Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                                                }
                                                                            });
                                                                        }
                                                                    },
                                                                    "✕ Cancel"
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
                        } else {
                            rsx! {
                                div { class: "grid grid-cols-1 md:grid-cols-3 gap-6",
                                    div { class: "p-5 rounded-xl border border-slate-800 bg-[#0c101d] space-y-2",
                                        div { class: "text-xs font-mono text-slate-400 uppercase", "Active Session Status" }
                                        div { class: "text-xl font-extrabold text-slate-400 flex items-center space-x-2 font-mono",
                                            span { "○ Idle (Ready)" }
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
                            }
                        }
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
                                                tr {
                                                    key: "{rid}",
                                                    class: if selected_run().as_ref().and_then(|sel| sel["id"].as_str()) == Some(&rid) {
                                                        "cursor-pointer bg-cyan-950/40 ring-1 ring-cyan-500/50 transition-all font-mono"
                                                    } else {
                                                        "cursor-pointer hover:bg-slate-800/60 transition-all font-mono"
                                                    },
                                                    onclick: {
                                                        let r_clone = r.clone();
                                                        let rid_c = rid.clone();
                                                        move |_| {
                                                            selected_run.set(Some(r_clone.clone()));
                                                            is_drawer_open.set(true);
                                                            let key_c = (state.api_key)();
                                                            let mut logs = drawer_logs;
                                                            let mut is_str = is_streaming_drawer;
                                                            logs.set(vec!["Following live run stream...".to_string()]);
                                                            let rid_sub = rid_c.clone();
                                                            spawn(async move {
                                                                is_str.set(true);
                                                                let _ = crate::api::stream_run(&key_c, &rid_sub, None, move |evt| {
                                                                    let mut list = logs();
                                                                    if let Some(c) = evt.content() {
                                                                        list.push(format!("[prose] {c}"));
                                                                    } else if let Some(t) = evt.tool_name() {
                                                                        list.push(format!("[tool] {t}"));
                                                                    } else if let Some(r) = evt.reasoning() {
                                                                        list.push(format!("[thought] {r}"));
                                                                    }
                                                                    if list.len() > 250 {
                                                                        list.remove(0);
                                                                    }
                                                                    logs.set(list);
                                                                }).await;
                                                                is_str.set(false);
                                                            });
                                                        }
                                                    },
                                                    td { class: "px-6 py-4 font-bold text-slate-200 group-hover:text-cyan-300", "{rid}" }
                                                    td { class: "px-6 py-4",
                                                        span { class: "px-2.5 py-1 rounded-full text-[11px] font-bold border {status_color} inline-flex items-center space-x-1.5 shadow-sm",
                                                            if is_running {
                                                                span { class: "w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" }
                                                            }
                                                            span { "{status}" }
                                                        }
                                                    }
                                                    td { class: "px-6 py-4 text-slate-400 truncate max-w-xs", "{conv}" }
                                                    td { class: "px-6 py-4 text-slate-400", "{created}" }
                                                    td { class: "px-6 py-4 text-right space-x-2",
                                                        button {
                                                            class: "px-3 py-1 rounded bg-cyan-900/60 hover:bg-cyan-700 text-cyan-200 border border-cyan-700/50 text-xs font-semibold cursor-pointer transition-colors",
                                                            "🔍 Inspect"
                                                        }
                                                        button {
                                                            class: "px-3 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-semibold cursor-pointer transition-colors",
                                                            onclick: move |e| {
                                                                e.stop_propagation();
                                                                let mut page = state.active_page;
                                                                page.set(SelectedPage::Chat);
                                                            },
                                                            "💬 Chat"
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

            // 4. Slide-Over Drawer for Subagent/Run Action & Stream
            if is_drawer_open() && let Some(run_val) = selected_run() {
                {
                    let run_id = run_val["id"].as_str().unwrap_or("").to_string();
                    let status = run_val["status"].as_str().unwrap_or("unknown").to_string();
                    let agent_id = run_val["agent_id"].as_str().unwrap_or("").to_string();
                    let run_id_steer = run_id.clone();
                    let run_id_swap = run_id.clone();
                    let run_id_cancel = run_id.clone();
                    let key_steer = (state.api_key)();
                    let key_swap = (state.api_key)();
                    let key_cancel = (state.api_key)();
                    let st = state;

                    rsx! {
                        div {
                            class: "fixed inset-0 bg-black/60 backdrop-blur-xs z-40 transition-opacity",
                            onclick: move |_| is_drawer_open.set(false),
                        }
                        div {
                            class: "fixed top-0 right-0 h-full w-full max-w-xl bg-[#090d16] border-l border-slate-800 shadow-2xl z-50 flex flex-col font-mono text-xs",
                            // Drawer Header
                            header { class: "px-6 py-4 border-b border-slate-800 flex items-center justify-between bg-[#0e1322]",
                                div { class: "space-y-0.5",
                                    h3 { class: "text-sm font-bold text-slate-100 uppercase tracking-wide flex items-center space-x-2",
                                        span { "Subagent Control Tray" }
                                    }
                                    div { class: "text-[11px] text-slate-400 flex items-center space-x-2",
                                        span { "ID: {run_id}" }
                                        span { "·" }
                                        span { class: "text-cyan-400 font-semibold", "{agent_id}" }
                                    }
                                }
                                button {
                                    class: "p-2 rounded-lg hover:bg-slate-800 text-slate-400 hover:text-slate-100 text-sm cursor-pointer",
                                    onclick: move |_| is_drawer_open.set(false),
                                    "✕"
                                }
                            }

                            // Drawer Action Bar
                            div { class: "p-5 border-b border-slate-800 bg-[#0b101c] space-y-4",
                                div { class: "space-y-1.5",
                                    label { class: "text-[10px] uppercase text-slate-400 font-bold", "Supervisor Steering Guidance" }
                                    div { class: "flex items-center space-x-2",
                                        input {
                                            class: "flex-1 px-3 py-1.5 rounded-lg bg-[#141926] border border-slate-700 text-slate-100 placeholder-slate-500 focus:outline-none focus:border-cyan-500",
                                            placeholder: "Inject steering guidance for next turn...",
                                            value: "{steer_input}",
                                            oninput: move |e| steer_input.set(e.value()),
                                        }
                                        button {
                                            class: "px-3.5 py-1.5 rounded-lg bg-cyan-600 hover:bg-cyan-500 text-white font-bold cursor-pointer transition-colors shadow",
                                            onclick: move |_| {
                                                let id = run_id_steer.clone();
                                                let msg = (steer_input)().clone();
                                                let k = key_steer.clone();
                                                let st_c = st;
                                                let mut input_sig = steer_input;
                                                spawn(async move {
                                                    let client = crate::api::CadeApiClient::new(k);
                                                    match client.steer_subagent(&id, &msg).await {
                                                        Ok(_) => {
                                                            add_toast(&st_c, ToastLevel::Success, "Steered", format!("Guidance sent to {id}"));
                                                            input_sig.set(String::new());
                                                        }
                                                        Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                    }
                                                });
                                            },
                                            "Send Guidance"
                                        }
                                    }

                                    // Quick guidance presets
                                    div { class: "flex items-center flex-wrap gap-1.5 pt-1",
                                        for preset in &["Focus on tests", "Skip file exploration", "Summarize progress", "Conclude task cleanly"] {
                                            {
                                                let preset_txt = preset.to_string();
                                                let mut input_sig = steer_input;
                                                rsx! {
                                                    button {
                                                        class: "px-2 py-0.5 rounded bg-slate-800/80 hover:bg-slate-700 text-[10px] text-slate-300 border border-slate-700/60 cursor-pointer transition-colors",
                                                        onclick: move |_| input_sig.set(preset_txt.clone()),
                                                        "+ {preset}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                div { class: "space-y-1.5",
                                    label { class: "text-[10px] uppercase text-slate-400 font-bold", "Model Hot-Swap (Next Turn)" }
                                    div { class: "flex items-center space-x-2",
                                        select {
                                            class: "flex-1 px-3 py-1.5 rounded-lg bg-[#141926] border border-slate-700 text-amber-300 font-mono text-xs focus:outline-none focus:border-amber-500 cursor-pointer",
                                            value: "{model_input}",
                                            onchange: move |e: Event<FormData>| model_input.set(e.value()),
                                            option { value: "gemini/gemini-2.0-flash", "gemini-2.0-flash (Fast · Default)" }
                                            option { value: "anthropic/claude-haiku-4-5", "claude-haiku-4-5 (Fast)" }
                                            option { value: "openai/o4-mini", "o4-mini (Fast)" }
                                            option { value: "anthropic/claude-3-7-sonnet", "claude-3-7-sonnet (Deep Reasoning)" }
                                            option { value: "gemini/gemini-2.5-pro", "gemini-2.5-pro (Deep Reasoning)" }
                                        }
                                        button {
                                            class: "px-3.5 py-1.5 rounded-lg bg-amber-600 hover:bg-amber-500 text-white font-bold cursor-pointer transition-colors shadow",
                                            onclick: move |_| {
                                                let id = run_id_swap.clone();
                                                let m = (model_input)().clone();
                                                let k = key_swap.clone();
                                                let st_c = st;
                                                spawn(async move {
                                                    let client = crate::api::CadeApiClient::new(k);
                                                    match client.swap_subagent_model(&id, &m).await {
                                                        Ok(_) => {
                                                            add_toast(&st_c, ToastLevel::Success, "Model Swapped", format!("Swapped {id} to {m}"));
                                                        }
                                                        Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                    }
                                                });
                                            },
                                            "Hot-Swap"
                                        }
                                    }
                                }

                                div { class: "flex items-center justify-between pt-1",
                                    span { class: "text-slate-400 text-[11px]", "Status: " span { class: "text-slate-200 font-bold uppercase", "{status}" } }
                                    button {
                                        class: "px-3 py-1 rounded bg-rose-900/60 hover:bg-rose-800 text-rose-200 border border-rose-700/50 cursor-pointer text-xs font-semibold",
                                        onclick: move |_| {
                                            let id = run_id_cancel.clone();
                                            let k = key_cancel.clone();
                                            let st_c = st;
                                            spawn(async move {
                                                let client = crate::api::CadeApiClient::new(k);
                                                match client.cancel_run(&id).await {
                                                    Ok(_) => add_toast(&st_c, ToastLevel::Warning, "Cancelled", format!("Run {id} cancelled")),
                                                    Err(e) => add_toast(&st_c, ToastLevel::Error, "Failed", e),
                                                }
                                            });
                                        },
                                        "Cancel Task"
                                    }
                                }
                            }

                            // Drawer Streaming Output
                            div { class: "p-3 bg-[#0c101c] border-b border-slate-800 flex items-center justify-between text-[11px] text-slate-400",
                                span { "Live Streaming Activity Log" }
                                if is_streaming_drawer() {
                                    span { class: "text-emerald-400 animate-pulse flex items-center space-x-1.5",
                                        span { class: "w-1.5 h-1.5 rounded-full bg-emerald-400" }
                                        span { "Streaming" }
                                    }
                                }
                            }
                            div { class: "flex-1 overflow-y-auto p-4 bg-[#050811] space-y-1.5 font-mono text-xs select-text",
                                for (idx, line) in drawer_logs().iter().enumerate() {
                                    {
                                        if line.starts_with("[thought]") {
                                            let text = line.strip_prefix("[thought] ").unwrap_or(line);
                                            rsx! {
                                                div { key: "{idx}", class: "text-[11px] text-amber-300/90 italic bg-amber-950/20 border border-amber-900/30 rounded p-1.5 break-words whitespace-pre-wrap leading-relaxed",
                                                    span { class: "font-bold font-sans not-italic text-amber-400 mr-1.5 uppercase text-[9px]", "Thinking:" }
                                                    "{text}"
                                                }
                                            }
                                        } else if line.starts_with("[tool]") {
                                            let text = line.strip_prefix("[tool] ").unwrap_or(line);
                                            rsx! {
                                                div { key: "{idx}", class: "text-[11px] text-cyan-300 bg-cyan-950/30 border border-cyan-800/40 rounded p-1.5 break-words font-semibold",
                                                    span { class: "text-cyan-400 mr-1.5 uppercase text-[9px] font-bold", "Tool Call:" }
                                                    "{text}"
                                                }
                                            }
                                        } else {
                                            let text = line.strip_prefix("[prose] ").unwrap_or(line);
                                            rsx! {
                                                div { key: "{idx}", class: "text-[11px] text-slate-200 break-words whitespace-pre-wrap leading-relaxed py-0.5",
                                                    "{text}"
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

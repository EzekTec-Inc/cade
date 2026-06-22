//! Properly implemented pages for sidebar items, plus placeholder stubs for
//! features that need server-side endpoints not yet available.

use dioxus::prelude::*;

use crate::api;
use crate::types::{add_toast, AppState, SelectedPage, ToastLevel};

// ── Agents page ───────────────────────────────────────────────────────────

#[component]
pub fn AgentsView() -> Element {
    let state = use_context::<AppState>();
    let agents = use_signal(Vec::<cade_api_types::AgentInfo>::new);
    let fetching = use_signal(|| true);

    let key = state.api_key;
    use_effect(move || {
        let k = key;
        let mut ags = agents;
        let mut busy = fetching;
        let st = state;
        spawn(async move {
            match api::list_agents(&k()).await {
                Ok(list) => ags.set(list),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch agents", e),
            }
            busy.set(false);
        });
    });

    let items: Vec<cade_api_types::AgentInfo> = agents().clone();

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Agents" }
            }
            div { class: "p-10 space-y-4",
                h2 { class: "text-sm font-semibold text-white", "Configured Agents" }
                if fetching() {
                    for _ in 0..3 {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/4 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-1/2 mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-1/3" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                        p { class: "text-gray-500 text-sm", "No agents configured." }
                    }
                } else {
                    {items.into_iter().map(|a| {
                        let name = a.name.clone();
                        let model = a.model.clone().unwrap_or_else(|| "\u{2014}".to_string());
                        let provider = a.provider.clone().unwrap_or_else(|| "\u{2014}".to_string());
                        rsx! {
                            div {
                                class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 flex items-center justify-between hover:border-[#373840] transition cursor-pointer",
                                onclick: move |_| {
                                    let mut st = use_context::<AppState>();
                                    st.selected_agent.set(Some(a.clone()));
                                    st.active_page.set(SelectedPage::Chat);
                                },
                                div { class: "flex flex-col space-y-1",
                                    div { class: "flex items-center space-x-3",
                                        span { class: "text-white font-semibold text-sm", "{name}" }
                                    }
                                    div { class: "flex items-center space-x-3 text-xs text-gray-400",
                                        span { "{provider}" }
                                        span { "\u{2022}" }
                                        span { class: "font-mono text-gray-500", "{model}" }
                                    }
                                }
                                span { class: "text-gray-500 text-xs", "\u{2192} Chat" }
                            }
                        }
                    })}
                }
            }
        }
    }
}

// ── Memory Blocks page ────────────────────────────────────────────────────

#[component]
pub fn MemoryBlocksView() -> Element {
    let state = use_context::<AppState>();
    let blocks = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)().map(|a| a.id.clone()).unwrap_or_default();

    let key = state.api_key;
    use_effect(move || {
        let aid = agent_id.clone();
        let k = key;
        let st = state;
        let mut blks = blocks;
        let mut busy = fetching;
        spawn(async move {
            let actual = if aid.is_empty() {
                api::list_agents(&k()).await.ok()
                    .and_then(|list| list.into_iter().next())
                    .map(|a| a.id)
                    .unwrap_or_default()
            } else {
                aid
            };
            if !actual.is_empty() {
                match api::list_memory_blocks(&actual, &k()).await {
                    Ok(data) => blks.set(data),
                    Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch memory blocks", e),
                }
            }
            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = blocks().clone();

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Memory Blocks" }
            }
            div { class: "p-10 space-y-4",
                h2 { class: "text-sm font-semibold text-white", "Agent Memory" }
                if fetching() {
                    for _ in 0..3 {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-full mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-2/3" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                        p { class: "text-gray-500 text-sm", "No memory blocks found for this agent." }
                    }
                } else {
                    {items.into_iter().map(|b| {
                        let label = b.get("label").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let value = b.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let tier = b.get("tier").and_then(|v| v.as_str()).unwrap_or("short").to_string();
                        let tier_color = match tier.as_str() {
                            "pinned" => "text-purple-400",
                            "long" => "text-blue-400",
                            _ => "text-gray-400",
                        };
                        rsx! {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5",
                                div { class: "flex items-center justify-between mb-2",
                                    span { class: "text-white font-semibold text-sm", "{label}" }
                                    span { class: "text-[10px] font-bold {tier_color} uppercase tracking-wider", "{tier}" }
                                }
                                p { class: "text-gray-300 text-xs whitespace-pre-wrap font-mono line-clamp-4", "{value}" }
                            }
                        }
                    })}
                }
            }
        }
    }
}

// ── Tools page (MCP + Tools) ──────────────────────────────────────────────

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

// ── Models page ───────────────────────────────────────────────────────────

#[component]
pub fn ModelsView() -> Element {
    let state = use_context::<AppState>();
    let models_data = use_signal(|| None::<serde_json::Value>);
    let fetching = use_signal(|| true);

    let key = state.api_key;
    use_effect(move || {
        let k = key;
        let st = state;
        let mut md = models_data;
        let mut busy = fetching;
        spawn(async move {
            match api::list_models(&k()).await {
                Ok(data) => md.set(Some(data)),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch models", e),
            }
            busy.set(false);
        });
    });

    let supported: Vec<serde_json::Value> = models_data()
        .as_ref()
        .and_then(|v| v.get("supported"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let dynamic: Vec<serde_json::Value> = models_data()
        .as_ref()
        .and_then(|v| v.get("dynamic"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Models" }
            }
            div { class: "p-10 space-y-6",
                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Available Models" }
                    if fetching() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-1/2" }
                        }
                    } else if supported.is_empty() && dynamic.is_empty() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                            p { class: "text-gray-500 text-sm", "No models available. Configure a provider first." }
                        }
                    } else {
                        {supported.into_iter().map(|m| {
                            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let context = m.get("context_length").and_then(|v| v.as_u64()).unwrap_or(0);
                            rsx! {
                                div { class: "bg-[#16171d] border border-[#272833] rounded-xl px-5 py-4 flex items-center justify-between",
                                    div { class: "flex flex-col",
                                        span { class: "text-white text-sm font-medium", "{id}" }
                                        if !provider.is_empty() {
                                            span { class: "text-gray-500 text-xs", "{provider}" }
                                        }
                                    }
                                    if context > 0 {
                                        span { class: "text-gray-400 text-xs", "{context / 1000}K context" }
                                    }
                                }
                            }
                        })}
                        if !dynamic.is_empty() {
                            div { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase pt-2", "Dynamic / Available" }
                            {dynamic.into_iter().map(|m| {
                                let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                rsx! {
                                    div { class: "bg-[#16171d]/40 border border-[#272833]/60 rounded-xl px-5 py-3 flex items-center justify-between",
                                        div { class: "flex flex-col",
                                            span { class: "text-gray-300 text-sm", "{id}" }
                                            if !provider.is_empty() {
                                                span { class: "text-gray-500 text-xs", "{provider}" }
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
}

// ── Logs page (agent events) ──────────────────────────────────────────────

#[component]
pub fn LogsView() -> Element {
    let state = use_context::<AppState>();
    let events = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)().map(|a| a.id.clone()).unwrap_or_default();

    let key = state.api_key;
    use_effect(move || {
        let aid = agent_id.clone();
        let k = key;
        let st = state;
        let mut evts = events;
        let mut busy = fetching;
        spawn(async move {
            if !aid.is_empty() {
                match api::list_events(&aid, &k()).await {
                    Ok(list) => evts.set(list),
                    Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch events", e),
                }
            }
            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = events().clone();

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Logs" }
            }
            div { class: "p-10 space-y-4",
                h2 { class: "text-sm font-semibold text-white", "Agent Events (last 50)" }
                if fetching() {
                    for _ in 0..5 {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-4 animate-pulse",
                            div { class: "h-3 bg-[#272833] rounded w-1/4 mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-3/4" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                        p { class: "text-gray-500 text-sm", "No events found." }
                    }
                } else {
                    {items.into_iter().map(|e| {
                        let event_type = e.get("event_type").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let ts = e.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
                        let detail = e.get("details").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        rsx! {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl px-5 py-3",
                                div { class: "flex items-center justify-between",
                                    span { class: "text-white text-xs font-medium", "{event_type}" }
                                    span { class: "text-gray-500 text-[10px]", "{ts}" }
                                }
                                if !detail.is_empty() {
                                    p { class: "text-gray-400 text-xs mt-1 line-clamp-2", "{detail}" }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

// ── ApiKeys page (informational — no server CRUD) ─────────────────────────

#[component]
pub fn ApiKeysView() -> Element {
    let state = use_context::<AppState>();
    let key = (state.api_key)();
    let masked = if key.len() > 8 {
        format!("{}\u{2026}{}", &key[..4], &key[key.len()-4..])
    } else {
        "(not set)".to_string()
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "API Keys" }
            }
            div { class: "p-10",
                div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 max-w-lg",
                    h2 { class: "text-white font-semibold text-sm mb-4", "Server Authentication" }
                    p { class: "text-gray-400 text-xs mb-4",
                        "CADE uses a single server-level bearer token for API authentication. \
                         Set the CADE_API_KEY environment variable or configure it in the server settings file. \
                         There is no runtime API key management \u{2014} the key is fixed at server startup."
                    }
                    div { class: "bg-[#1f212a] border border-[#272833] rounded-lg p-3",
                        div { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase mb-1", "Current Key" }
                        span { class: "text-gray-300 text-sm font-mono", "{masked}" }
                    }
                    p { class: "text-gray-600 text-xs mt-4",
                        "To change the key, restart the server with a new CADE_API_KEY value."
                    }
                }
            }
        }
    }
}

// ── Remaining stubs (no server endpoints) ─────────────────────────────────

macro_rules! stub_page {
    ($name:ident, $title:expr, $desc:expr) => {
        #[component]
        pub fn $name() -> Element {
            rsx! {
                div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
                    header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                        h1 { class: "text-lg font-semibold text-white", $title }
                    }
                    div { class: "p-10",
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center max-w-lg mx-auto",
                            div { class: "text-4xl mb-4", "\u{1f6a7}" }
                            h2 { class: "text-white font-semibold text-lg mb-2", $title }
                            p { class: "text-gray-400 text-sm", $desc }
                        }
                    }
                }
            }
        }
    };
}

stub_page!(CodeView, "Code", "API playground and code snippets \u{2014} coming soon.");
stub_page!(UsageView, "Usage", "Token usage statistics and billing overview \u{2014} coming soon.");
stub_page!(SettingsView, "Settings", "Application preferences and configuration \u{2014} coming soon.");

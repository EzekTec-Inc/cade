mod api;
mod components;
mod types;

use dioxus::prelude::*;

use types::{AppState, SelectedPage};

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    LaunchBuilder::new()
        .with_cfg(dioxus::web::Config::new().rootname("cade_gui_canvas"))
        .launch(App);
}

#[component]
fn App() -> Element {
    // ── Extract query parameters from window.location (Cross-Frontend Sync) ──
    let mut initial_key = String::new();
    let mut initial_agent_id = Option::<String>::None;
    let mut initial_conv_id = Option::<String>::None;

    if let Some(window) = web_sys::window()
        && let Ok(search) = window.location().search()
            && !search.is_empty() {
                let query = search.trim_start_matches('?');
                for pair in query.split('&') {
                    let parts: Vec<&str> = pair.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0];
                        let val = urlencoding::decode(parts[1])
                            .unwrap_or_default()
                            .into_owned();
                        match key {
                            "api_key" => initial_key = val,
                            "agent_id" => initial_agent_id = Some(val),
                            "conversation_id" => initial_conv_id = Some(val),
                            _ => {}
                        }
                    }
                }
            }

    let initial_page = if initial_agent_id.is_some() {
        SelectedPage::Chat
    } else {
        SelectedPage::Dashboard
    };

    // ── Shared state ────────────────────────────────────────────────────────
    let api_key = use_signal(|| initial_key);
    let mut active_page = use_signal(|| initial_page);
    let selected_agent: Signal<Option<cade_api_types::AgentInfo>> = use_signal(|| {
        initial_agent_id.map(|id| cade_api_types::AgentInfo {
            id,
            name: "Agent".to_string(),
            model: Some("unknown".to_string()),
            provider: None,
            theme: None,
        })
    });
    let messages = use_signal(Vec::<cade_api_types::ChatMessage>::new);
    let input_text = use_signal(String::new);
    let is_loading = use_signal(|| false);
    let conversations = use_signal(Vec::<cade_api_types::ConversationInfo>::new);
    let active_conversation = use_signal(|| initial_conv_id);
    let toasts = use_signal(Vec::<types::ToastMessage>::new);
    let global_error = use_signal(|| Option::<String>::None);
    let active_stream_id = use_signal(|| Option::<String>::None);
    let active_stream = use_signal(types::SafeAbortHandle::default);
    let parsed_messages =
        use_signal(std::collections::HashMap::<String, (String, Option<String>)>::new);

    // Provide individual signals and composite state to all children
    use_context_provider(|| api_key);
    use_context_provider(|| active_page);
    use_context_provider(|| selected_agent);
    use_context_provider(|| messages);
    use_context_provider(|| input_text);
    use_context_provider(|| is_loading);
    use_context_provider(|| conversations);
    use_context_provider(|| active_conversation);
    use_context_provider(|| toasts);
    use_context_provider(|| global_error);
    use_context_provider(|| active_stream_id);
    use_context_provider(|| active_stream);
    use_context_provider(|| parsed_messages);

    let app_state = AppState {
        api_key,
        active_page,
        selected_agent,
        messages,
        input_text,
        is_loading,
        conversations,
        active_conversation,
        toasts,
        global_error,
        active_stream_id,
        active_stream,
        parsed_messages,
    };
    use_context_provider(|| app_state);

    let client = use_memo(move || crate::api::CadeApiClient::new(api_key()));
    use_context_provider(|| client);

    let store = use_memo(move || crate::types::AppSessionStore::new(app_state));
    use_context_provider(|| store);

    // ── Startup: fetch first agent + start real-time SSE event loop ─────────
    use_effect(move || {
        let key = api_key;
        let _state = app_state;
        let mut selected = selected_agent;
        let mut convs = conversations;
        let mut messages = messages;
        let mut active_conversation = active_conversation;
        let mut global_error = global_error;

        spawn(async move {
            // Wait until an API key is configured
            while key().is_empty() {
                gloo_timers::future::TimeoutFuture::new(200).await;
            }

            // Fetch initial agent + conversations (silent poll; show toast only on failure)
            match api::list_agents(&key()).await {
                Ok(list) => {
                    let matched = if let Some(ref initial_agent) = *selected.peek() {
                        list.iter().find(|a| a.id == initial_agent.id).cloned()
                    } else {
                        None
                    };

                    if let Some(agent) = matched.or_else(|| list.into_iter().next()) {
                        let agent_id = agent.id.clone();
                        selected.set(Some(agent));
                        let _ = api::list_conversations(&agent_id, &key())
                            .await
                            .map(|list| convs.set(list));
                    }
                }
                Err(e) => {
                    global_error.set(Some(e.clone()));
                }
            }

            // Real-time SSE event loop
            loop {
                let client_inst = crate::api::CadeApiClient::new(key());

                let sse_res = client_inst
                    .listen_global_events(|event| {
                        let event_type = event["event_type"].as_str().unwrap_or("");
                        match event_type {
                            "conversation_created" => {
                                let agent_id = event["agent_id"].as_str().unwrap_or("");
                                if let Some(curr) = selected()
                                    && curr.id == agent_id
                                        && let Ok(conv) = serde_json::from_value::<
                                            cade_api_types::ConversationInfo,
                                        >(
                                            event["conversation"].clone()
                                        ) {
                                            let mut list = convs();
                                            if !list.contains(&conv) {
                                                list.push(conv);
                                                convs.set(list);
                                            }
                                        }
                            }
                            "conversation_deleted" => {
                                let agent_id = event["agent_id"].as_str().unwrap_or("");
                                let conv_id = event["conversation_id"].as_str().unwrap_or("");
                                if let Some(curr) = selected()
                                    && curr.id == agent_id {
                                        let mut list = convs();
                                        list.retain(|c| c.id != conv_id);
                                        convs.set(list);
                                        if active_conversation() == Some(conv_id.to_string()) {
                                            active_conversation.set(None);
                                        }
                                    }
                            }
                            "message_created" => {
                                let m_agent_id = event["agent_id"].as_str().unwrap_or("");
                                let m_conv_id = event["conversation_id"].as_str();
                                if let Some(curr_agent) = selected()
                                    && curr_agent.id == m_agent_id
                                        && active_conversation() == m_conv_id.map(String::from)
                                        && let Ok(msg) =
                                            serde_json::from_value::<cade_api_types::ChatMessage>(
                                                event["message"].clone(),
                                            )
                                        {
                                            let mut list = messages();
                                            if !list.iter().any(|m| m.id == msg.id) {
                                                list.push(msg);
                                                messages.set(list);
                                            }
                                        }
                            }
                            _ => {}
                        }
                    })
                    .await;

                if let Err(e) = sse_res {
                    global_error.set(Some(format!("Server connection lost: {e}")));
                    gloo_timers::future::TimeoutFuture::new(3000).await;

                    // Re-sync on reconnect
                    if let Ok(list) = api::list_agents(&key()).await {
                        global_error.set(None);
                        if let Some(first) = list.into_iter().next() {
                            let agent_id = first.id.clone();
                            selected.set(Some(first));
                            if let Ok(c_list) = api::list_conversations(&agent_id, &key()).await {
                                convs.set(c_list);
                            }
                        }
                    }
                }
            }
        });
    });

    // ── Render ──────────────────────────────────────────────────────────────
    rsx! {
        div {
            class: "w-screen h-screen flex bg-[#0f1115] text-gray-200 overflow-hidden",
            // Keyboard shortcuts:
            //   Ctrl+N   → Chat
            //   Ctrl+,   → Settings
            //   Escape   → Chat (if not already there)
            onkeydown: move |e| {
                if e.key() == Key::Character("n".into()) && e.modifiers().ctrl() {
                    active_page.set(SelectedPage::Chat);
                } else if e.key() == Key::Character(",".into()) && e.modifiers().ctrl() {
                    active_page.set(SelectedPage::Settings);
                } else if e.key() == Key::Escape && (active_page)() != SelectedPage::Chat {
                    active_page.set(SelectedPage::Chat);
                }
            },
            if (api_key)().is_empty() {
                components::login::LoginScreen {}
            } else {
                if let Some(err) = (global_error)() {
                    div { class: "fixed inset-0 bg-[#0f1115]/95 z-50 flex flex-col items-center justify-center p-6 text-center select-none",
                        div { class: "bg-[#16171d] border border-red-500/50 rounded-2xl p-10 max-w-md mx-auto shadow-2xl",
                            div { class: "text-red-500 text-5xl mb-6", "⚠️" }
                            h2 { class: "text-white font-semibold text-xl mb-3", "CADE Server Offline" }
                            p { class: "text-gray-400 text-sm mb-6", "{err}" }
                            div { class: "flex items-center justify-center gap-3 text-sm text-[#5d6175]",
                                span { class: "w-4 h-4 rounded-full border-2 border-t-[#00c8ff] border-[#272833] animate-spin" }
                                span { "Attempting to reconnect..." }
                            }
                        }
                    }
                }
                components::sidebar::Sidebar {}
                main { class: "flex-1 bg-[#0f1115] overflow-y-auto flex flex-col justify-between h-full select-text pb-8",
                    if (active_page)() == SelectedPage::Chat {
                        components::chat::ChatView {}
                    } else if (active_page)() == SelectedPage::Providers {
                        components::providers::ProvidersView {}
                    } else if (active_page)() == SelectedPage::Code {
                        components::code::CodeView {}
                    } else if (active_page)() == SelectedPage::Agents {
                        components::agents::AgentsView {}
                    } else if (active_page)() == SelectedPage::Logs {
                        components::logs_page::LogsView {}
                    } else if (active_page)() == SelectedPage::MemoryBlocks {
                        components::memory::MemoryBlocksView {}
                    } else if (active_page)() == SelectedPage::Tools {
                        components::tools_page::ToolsView {}
                    } else if (active_page)() == SelectedPage::Models {
                        components::models_page::ModelsView {}
                    } else if (active_page)() == SelectedPage::ApiKeys {
                        components::api_keys::ApiKeysView {}
                    } else if (active_page)() == SelectedPage::Usage {
                        components::usage::UsageView {}
                    } else if (active_page)() == SelectedPage::Settings {
                        components::settings::SettingsView {}
                    } else {
                        components::dashboard::DashboardView {}
                    }
                }
                components::toast::ToastContainer {}
            }
        }
    }
}

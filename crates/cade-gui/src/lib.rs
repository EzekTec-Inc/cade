mod api;
mod components;
mod types;

use dioxus::prelude::*;

use types::{add_toast, AppState, SelectedPage, ToastLevel};

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    LaunchBuilder::new()
        .with_cfg(dioxus::web::Config::new().rootname("cade_gui_canvas"))
        .launch(App);
}

#[component]
fn App() -> Element {
    // ── Shared state ────────────────────────────────────────────────────────
    let api_key = use_signal(String::new);
    let mut active_page = use_signal(|| SelectedPage::Dashboard);
    let selected_agent: Signal<Option<cade_api_types::AgentInfo>> = use_signal(|| None);
    let messages = use_signal(Vec::<cade_api_types::ChatMessage>::new);
    let input_text = use_signal(String::new);
    let is_loading = use_signal(|| false);
    let conversations = use_signal(Vec::<cade_api_types::ConversationInfo>::new);
    let active_conversation = use_signal(|| Option::<String>::None);
    let toasts = use_signal(Vec::<types::ToastMessage>::new);

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
    };
    use_context_provider(|| app_state);

    // ── Startup: fetch first agent + start message polling ─────────────────
    use_effect(move || {
        let key = api_key;
        let state = app_state;
        let mut selected = selected_agent;
        let mut msgs = messages;
        let mut convs = conversations;
        let active_conv = active_conversation;
        spawn(async move {
            // Wait until an API key is configured
            while key().is_empty() {
                gloo_timers::future::TimeoutFuture::new(200).await;
            }

            // Silently poll — show a toast only for the initial agent fetch
            match api::list_agents(&key()).await {
                Ok(list) => {
                    if let Some(first) = list.into_iter().next() {
                        selected.set(Some(first));
                    }
                }
                Err(e) => add_toast(&state, ToastLevel::Error, "Failed to fetch agents", e),
            }

            // Poll messages and conversations every 1.5s (silent — no toasts on poll errors)
            loop {
                if let Some(agent) = selected() {
                    let _ = api::list_conversations(&agent.id, &key()).await.map(|list| convs.set(list));
                    let conv_id = active_conv();
                    let _ = api::get_messages(&agent.id, &key(), conv_id.as_deref()).await.map(|list| msgs.set(list));
                }
                gloo_timers::future::TimeoutFuture::new(1500).await;
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
                components::sidebar::Sidebar {}
                main { class: "flex-1 bg-[#0f1115] overflow-y-auto flex flex-col justify-between h-full select-text pb-8",
                    if (active_page)() == SelectedPage::Chat {
                        components::chat::ChatView {}
                    } else if (active_page)() == SelectedPage::Providers {
                        components::providers::ProvidersView {}
                    } else if (active_page)() == SelectedPage::Code {
                        components::stubs::CodeView {}
                    } else if (active_page)() == SelectedPage::Agents {
                        components::stubs::AgentsView {}
                    } else if (active_page)() == SelectedPage::Logs {
                        components::stubs::LogsView {}
                    } else if (active_page)() == SelectedPage::MemoryBlocks {
                        components::stubs::MemoryBlocksView {}
                    } else if (active_page)() == SelectedPage::Tools {
                        components::stubs::ToolsView {}
                    } else if (active_page)() == SelectedPage::Models {
                        components::stubs::ModelsView {}
                    } else if (active_page)() == SelectedPage::ApiKeys {
                        components::stubs::ApiKeysView {}
                    } else if (active_page)() == SelectedPage::Usage {
                        components::stubs::UsageView {}
                    } else if (active_page)() == SelectedPage::Settings {
                        components::stubs::SettingsView {}
                    } else {
                        components::dashboard::DashboardView {}
                    }
                }
                components::toast::ToastContainer {}
            }
        }
    }
}

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
    // ── Shared state ────────────────────────────────────────────────────────
    let api_key = use_signal(String::new);
    let active_page = use_signal(|| SelectedPage::Dashboard);
    let selected_agent = use_signal(|| Option::<cade_api_types::AgentInfo>::None);
    let messages = use_signal(Vec::<cade_api_types::ChatMessage>::new);
    let input_text = use_signal(String::new);
    let is_loading = use_signal(|| false);

    // Provide individual signals and composite state to all children
    use_context_provider(|| api_key);
    use_context_provider(|| active_page);
    use_context_provider(|| selected_agent);
    use_context_provider(|| messages);
    use_context_provider(|| input_text);
    use_context_provider(|| is_loading);

    let app_state = AppState {
        api_key,
        active_page,
        selected_agent,
        messages,
        input_text,
        is_loading,
    };
    use_context_provider(|| app_state);

    // ── Startup: fetch first agent + start message polling ─────────────────
    use_effect(move || {
        let key = api_key;
        let mut selected = selected_agent;
        let mut msgs = messages;
        spawn(async move {
            // Wait until an API key is configured
            while key().is_empty() {
                gloo_timers::future::TimeoutFuture::new(200).await;
            }

            // Fetch the first agent
            if let Ok(agent_list) = api::list_agents(&key()).await
                && let Some(first) = agent_list.into_iter().next()
            {
                selected.set(Some(first));
            }

            // Poll messages every 1.5s
            loop {
                if let Some(agent) = selected()
                    && let Ok(msg_list) = api::get_messages(&agent.id, &key()).await
                {
                    msgs.set(msg_list);
                }
                gloo_timers::future::TimeoutFuture::new(1500).await;
            }
        });
    });

    // ── Render ──────────────────────────────────────────────────────────────
    rsx! {
        div { class: "w-screen h-screen flex bg-[#0f1115] text-gray-200 overflow-hidden",
            if (api_key)().is_empty() {
                components::login::LoginScreen {}
            } else {
                components::sidebar::Sidebar {}
                main { class: "flex-1 bg-[#0f1115] overflow-y-auto flex flex-col justify-between h-full select-text pb-8",
                    if (active_page)() == SelectedPage::Chat {
                        components::chat::ChatView {}
                    } else {
                        components::dashboard::DashboardView {}
                    }
                }
            }
        }
    }
}

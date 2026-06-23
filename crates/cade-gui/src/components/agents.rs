use dioxus::prelude::*;

use crate::api;
use crate::types::{add_toast, AppState, SelectedPage, ToastLevel};

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

use dioxus::prelude::*;

use crate::api;
use crate::types::{add_toast, AppState, ToastLevel};

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

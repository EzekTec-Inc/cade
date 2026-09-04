use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn LogsView() -> Element {
    let state = use_context::<AppState>();
    let events = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)()
        .map(|a| a.id.clone())
        .unwrap_or_default();

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
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70",
                h1 { class: "text-lg font-semibold text-slate-100", "Logs" }
            }
            div { class: "p-10 space-y-4",
                h2 { class: "text-sm font-semibold text-slate-100", "Agent Events (last 50)" }
                if fetching() {
                    for _ in 0..5 {
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-4 animate-pulse",
                            div { class: "h-3 bg-[#272833] rounded w-1/4 mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-3/4" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center",
                        p { class: "text-slate-500 text-sm", "No events found." }
                    }
                } else {
                    {items.into_iter().map(|e| {
                        let event_type = e.get("event_type").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let ts = e.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
                        let detail = e.get("details").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        rsx! {
                            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl px-5 py-3",
                                div { class: "flex items-center justify-between",
                                    span { class: "text-slate-100 text-xs font-medium", "{event_type}" }
                                    span { class: "text-slate-500 text-[10px]", "{ts}" }
                                }
                                if !detail.is_empty() {
                                    p { class: "text-slate-400 text-xs mt-1 line-clamp-2", "{detail}" }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn MemoryBlocksView() -> Element {
    let state = use_context::<AppState>();
    let blocks = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)()
        .map(|a| a.id.clone())
        .unwrap_or_default();

    let key = state.api_key;
    use_effect(move || {
        let aid = agent_id.clone();
        let k = key;
        let st = state;
        let mut blks = blocks;
        let mut busy = fetching;
        spawn(async move {
            let actual = if aid.is_empty() {
                api::list_agents(&k())
                    .await
                    .ok()
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

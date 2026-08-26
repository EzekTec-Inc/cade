use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn MemoryBlocksView() -> Element {
    let state = use_context::<AppState>();
    let blocks = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)()
        .map(|a| a.id.clone())
        .unwrap_or_default();

    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let engine = crate::api_engine::ApiClientEngine::new(client);

    use_effect(move || {
        let aid = agent_id.clone();
        let st = state;
        let mut blks = blocks;
        let mut busy = fetching;
        let eng = engine.clone();
        spawn(async move {
            let actual = if aid.is_empty() {
                eng.fetch_agents()
                    .await
                    .value()
                    .and_then(|list| list.first())
                    .map(|a| a.id.clone())
                    .unwrap_or_default()
            } else {
                aid
            };
            if !actual.is_empty() {
                match eng.fetch_memory_blocks(&actual).await {
                    crate::api_engine::ResourceState::Ready(data) => blks.set(data),
                    crate::api_engine::ResourceState::Error(e) => {
                        add_toast(&st, ToastLevel::Error, "Failed to fetch memory blocks", e)
                    }
                    _ => {}
                }
            }
            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = blocks().clone();
    let model_name = (state.selected_agent)().and_then(|a| a.model.clone());

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70",
                h1 { class: "text-lg font-semibold text-slate-100", "Memory Blocks" }
            }
            div { class: "p-10 space-y-4",
                TokenHeatmapWidget { blocks: blocks, model_name: model_name }

                h2 { class: "text-sm font-semibold text-slate-100 pt-2", "Agent Memory" }
                if fetching() {
                    for _ in 0..3 {
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-full mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-2/3" }
                        }
                    }
                } else if items.is_empty() {
                    div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center",
                        p { class: "text-slate-500 text-sm", "No memory blocks found for this agent." }
                    }
                } else {
                    {items.into_iter().map(|b| {
                        let label = b.get("label").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let value = b.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let tier = b.get("tier").and_then(|v| v.as_str()).unwrap_or("short").to_string();
                        let tier_color = match tier.as_str() {
                            "pinned" => "text-purple-400",
                            "long" => "text-blue-400",
                            _ => "text-slate-400",
                        };
                        rsx! {
                            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5",
                                div { class: "flex items-center justify-between mb-2",
                                    span { class: "text-slate-100 font-semibold text-sm", "{label}" }
                                    span { class: "text-[10px] font-bold {tier_color} uppercase tracking-wider", "{tier}" }
                                }
                                p { class: "text-slate-300 text-xs whitespace-pre-wrap font-mono line-clamp-4", "{value}" }
                            }
                        }
                    })}
                }
            }
        }
    }
}

/// Dynamic visual context window allocation bar.
#[component]
pub fn TokenHeatmapWidget(blocks: Signal<Vec<serde_json::Value>>, model_name: Option<String>) -> Element {
    let raw_blocks = blocks();
    let model = model_name.unwrap_or_else(|| "claude-3-5-sonnet".to_string());

    let mut pinned_chars = 0usize;
    let mut short_chars = 0usize;
    let mut long_chars = 0usize;

    for b in &raw_blocks {
        let val_len = b.get("value").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(0);
        let tier = b.get("tier").and_then(|v| v.as_str()).unwrap_or("short");
        match tier {
            "pinned" => pinned_chars += val_len,
            "long" => long_chars += val_len,
            _ => short_chars += val_len,
        }
    }

    let total_chars = pinned_chars + short_chars + long_chars;
    let total_tokens = total_chars / 4;
    let context_limit_tokens: usize = 128_000;
    let context_limit_chars: usize = context_limit_tokens * 4;

    let pinned_pct = ((pinned_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let short_pct = ((short_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let long_pct = ((long_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let usage_pct = (total_chars as f64 / context_limit_chars as f64) * 100.0;
    let is_warning = usage_pct >= 70.0;

    rsx! {
        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 mb-6 space-y-4 shadow-lg",
            div { class: "flex items-center justify-between",
                div { class: "flex items-center space-x-2.5",
                    span { class: "text-slate-100 font-semibold text-sm", "Context Window Token Heatmap" }
                    span { class: "text-[11px] font-mono text-slate-400 px-2 py-0.5 bg-[#16171d] rounded border border-[#1e293b]", "{model}" }
                }
                div { class: "flex items-center space-x-3 text-xs font-mono",
                    span { class: if is_warning { "text-amber-400 font-semibold" } else { "text-slate-400" },
                        "{total_tokens} / {context_limit_tokens} tokens ({usage_pct:.1}%)"
                    }
                    if is_warning {
                        span { class: "bg-amber-500/10 text-amber-400 text-[10px] px-2 py-0.5 rounded-full border border-amber-500/20", "Consolidation Near" }
                    }
                }
            }

            div { class: "w-full bg-[#16171d] h-3 rounded-full overflow-hidden flex border border-[#1e293b]/60",
                if pinned_chars > 0 {
                    div {
                        style: "width: {pinned_pct}%;",
                        class: "bg-gradient-to-r from-purple-500 to-indigo-500 h-full transition-all duration-300",
                        title: "Pinned Memory: {pinned_chars} chars"
                    }
                }
                if short_chars > 0 {
                    div {
                        style: "width: {short_pct}%;",
                        class: "bg-gradient-to-r from-cyan-500 to-sky-500 h-full transition-all duration-300",
                        title: "Active Short-Term: {short_chars} chars"
                    }
                }
                if long_chars > 0 {
                    div {
                        style: "width: {long_pct}%;",
                        class: "bg-gradient-to-r from-slate-500 to-slate-600 h-full transition-all duration-300",
                        title: "Archival Excerpts: {long_chars} chars"
                    }
                }
            }

            div { class: "flex items-center justify-between text-[11px] text-slate-400 pt-1 select-none",
                div { class: "flex items-center space-x-4",
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-purple-500" }
                        span { "Pinned ({pinned_chars / 4} tok)" }
                    }
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-cyan-500" }
                        span { "Active ({short_chars / 4} tok)" }
                    }
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-slate-500" }
                        span { "Archival ({long_chars / 4} tok)" }
                    }
                }
                span { class: "text-slate-500 font-mono text-[10px]", "Threshold @ 70%" }
            }
        }
    }
}

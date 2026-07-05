use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn UsageView() -> Element {
    let state = use_context::<AppState>();
    let agent_id = (state.selected_agent)()
        .map(|a| a.id.clone())
        .unwrap_or_default();
    let agent_name = (state.selected_agent)()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "—".to_string());

    let metrics_data = use_signal(|| Option::<serde_json::Value>::None);
    let context_data = use_signal(|| Option::<serde_json::Value>::None);
    let fetching = use_signal(|| true);

    let key = state.api_key;
    use_effect(move || {
        let aid = agent_id.clone();
        let k = key;
        let st = state;
        let mut metrics = metrics_data;
        let mut ctx = context_data;
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
                match api::get_metrics(&actual, &k()).await {
                    Ok(m) => metrics.set(Some(m)),
                    Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch metrics", e),
                }
                match api::get_context_stats(&actual, &k()).await {
                    Ok(c) => ctx.set(Some(c)),
                    Err(e) => {
                        if !e.contains("404") && !e.contains("not found") {
                            add_toast(&st, ToastLevel::Warning, "Context stats unavailable", e);
                        }
                    }
                }
            }
            busy.set(false);
        });
    });

    let body_content = if fetching() {
        rsx! {
            div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                for _ in 0..3 {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 animate-pulse",
                        div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                        div { class: "h-8 bg-[#272833] rounded w-1/2" }
                    }
                }
            }
        }
    } else {
        let metrics_section = metrics_data().as_ref().map(|m| {
            let input_tokens = m.get("input_tokens_total").and_then(|v| v.as_u64()).unwrap_or(0);
            let output_tokens = m.get("output_tokens_total").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_read = m.get("cache_read_tokens_total").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_write = m.get("cache_write_tokens_total").and_then(|v| v.as_u64()).unwrap_or(0);
            let consolidation_runs = m.get("consolidation_runs").and_then(|v| v.as_u64()).unwrap_or(0);
            let chars_summarised = m.get("chars_summarised").and_then(|v| v.as_u64()).unwrap_or(0);
            let chars_produced = m.get("chars_produced").and_then(|v| v.as_u64()).unwrap_or(0);
            let total_tokens = input_tokens + output_tokens + cache_read + cache_write;

            rsx! {
                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Token Usage (Lifetime)" }
                    div { class: "grid grid-cols-1 md:grid-cols-4 gap-4",
                        metric_card { label: "Input Tokens", value: format_tokens(input_tokens), color: "text-blue-400" }
                        metric_card { label: "Output Tokens", value: format_tokens(output_tokens), color: "text-emerald-400" }
                        metric_card { label: "Cache Read", value: format_tokens(cache_read), color: "text-purple-400" }
                        metric_card { label: "Cache Write", value: format_tokens(cache_write), color: "text-yellow-400" }
                    }
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6",
                        div { class: "flex items-center justify-between",
                            span { class: "text-sm font-semibold text-white", "Total Tokens" }
                            span { class: "text-lg font-bold text-white", "{format_tokens(total_tokens)}" }
                        }
                    }
                }
                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Consolidation" }
                    div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                        metric_card { label: "Consolidation Runs", value: consolidation_runs.to_string(), color: "text-gray-300" }
                        metric_card { label: "Chars Summarised", value: format_chars(chars_summarised), color: "text-gray-300" }
                        metric_card { label: "Chars Produced", value: format_chars(chars_produced), color: "text-gray-300" }
                    }
                }
            }
        });

        let context_section = context_data().as_ref().map(|c| {
            let model = c.get("model").and_then(|v| v.as_str()).unwrap_or("—").to_string();
            let window_tokens = c.get("window_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let input_budget_chars = c.get("input_budget_chars").and_then(|v| v.as_u64()).unwrap_or(0);
            let system_overhead_chars = c.get("system_overhead_chars").and_then(|v| v.as_u64()).unwrap_or(0);
            let system_tokens = c.get("system_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

            rsx! {
                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Context Window" }
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 space-y-4",
                        div { class: "flex items-center justify-between",
                            span { class: "text-gray-400 text-xs", "Model" }
                            span { class: "text-white text-xs font-mono", "{model}" }
                        }
                        div { class: "flex items-center justify-between",
                            span { class: "text-gray-400 text-xs", "Window Size" }
                            span { class: "text-white text-xs font-mono", "{window_tokens / 1000}K tokens" }
                        }
                        div { class: "flex items-center justify-between",
                            span { class: "text-gray-400 text-xs", "Input Budget" }
                            span { class: "text-white text-xs font-mono", "{format_chars(input_budget_chars)}" }
                        }
                        div { class: "flex items-center justify-between",
                            span { class: "text-gray-400 text-xs", "System Overhead (chars)" }
                            span { class: "text-white text-xs font-mono", "{format_chars(system_overhead_chars)}" }
                        }
                        div { class: "flex items-center justify-between",
                            span { class: "text-gray-400 text-xs", "System Overhead (tokens)" }
                            span { class: "text-white text-xs font-mono", "{system_tokens} tokens" }
                        }
                    }
                }
            }
        });

        rsx! {
            {metrics_section}
            {context_section}
            if metrics_data().is_none() && context_data().is_none() {
                div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                    p { class: "text-gray-500 text-sm", "No usage data available yet." }
                    p { class: "text-gray-600 text-xs mt-1", "Send some messages to an agent to see metrics." }
                }
            }
        }
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Usage & Metrics" }
                span { class: "text-xs text-gray-500", "{agent_name}" }
            }

            div { class: "p-10 space-y-6",
                {body_content}
            }
        }
    }
}

#[component]
fn metric_card(label: String, value: String, color: String) -> Element {
    rsx! {
        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 space-y-2",
            span { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "{label}" }
            div { class: "text-xl font-bold {color}", "{value}" }
        }
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_chars(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

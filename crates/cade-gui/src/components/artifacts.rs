//! Live Artifact Studio & Interactive Diff/Data Explorer (PRD #128 / Issue #131).

use dioxus::prelude::*;
use crate::types::AppState;

#[derive(Clone, PartialEq)]
pub enum ArtifactType {
    CodeDiff,
    TableData,
    MarkdownDoc,
    JsonPayload,
}

#[derive(Clone, PartialEq)]
pub struct ArtifactItem {
    pub id: String,
    pub title: String,
    pub artifact_type: ArtifactType,
    pub content: String,
    pub size_bytes: usize,
}

#[component]
pub fn ArtifactStudioView() -> Element {
    let state = use_context::<AppState>();
    let mut selected_tab = use_signal(|| 0usize);
    let mut filter_query = use_signal(String::new);

    // Extract artifacts from active messages
    let msgs = (state.messages)();
    let mut detected_artifacts = Vec::<ArtifactItem>::new();

    for (i, m) in msgs.iter().enumerate() {
        let text = match &m.content {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        if text.contains("```diff") || (text.contains("--- ") && text.contains("+++ ")) {
            detected_artifacts.push(ArtifactItem {
                id: format!("art-diff-{i}"),
                title: format!("Code Patch #{}", i + 1),
                artifact_type: ArtifactType::CodeDiff,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.contains("```json") || (text.starts_with('{') && text.ends_with('}')) {
            detected_artifacts.push(ArtifactItem {
                id: format!("art-json-{i}"),
                title: format!("Structured Data #{}", i + 1),
                artifact_type: ArtifactType::JsonPayload,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.contains('|') && text.contains("---") {
            detected_artifacts.push(ArtifactItem {
                id: format!("art-table-{i}"),
                title: format!("Tabular Dataset #{}", i + 1),
                artifact_type: ArtifactType::TableData,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.len() > 300 {
            detected_artifacts.push(ArtifactItem {
                id: format!("art-doc-{i}"),
                title: format!("Documentation Note #{}", i + 1),
                artifact_type: ArtifactType::MarkdownDoc,
                content: text.clone(),
                size_bytes: text.len(),
            });
        }
    }

    if detected_artifacts.is_empty() {
        detected_artifacts.push(ArtifactItem {
            id: "sample-diff-1".to_string(),
            title: "crates/cade-gui/src/components/chat.rs (Patch)".to_string(),
            artifact_type: ArtifactType::CodeDiff,
            content: "--- a/crates/cade-gui/src/components/chat.rs\n+++ b/crates/cade-gui/src/components/chat.rs\n@@ -114,3 +114,5 @@\n+    // Reactive session hydration\n+    use_effect(move || { ... });".to_string(),
            size_bytes: 184,
        });
        detected_artifacts.push(ArtifactItem {
            id: "sample-table-1".to_string(),
            title: "Model Benchmark Telemetry (CSV/Table)".to_string(),
            artifact_type: ArtifactType::TableData,
            content: "| Model | Provider | Context Window | TTFT (ms) |\n|---|---|---|---|\n| claude-3-5-sonnet | Anthropic | 200k | 240 |\n| gpt-4o | OpenAI | 128k | 210 |\n| llama-3.3-70b | Ollama | 128k | 380 |".to_string(),
            size_bytes: 236,
        });
    }

    let query = filter_query().to_lowercase();
    let filtered_artifacts: Vec<ArtifactItem> = detected_artifacts
        .into_iter()
        .filter(|a| query.is_empty() || a.title.to_lowercase().contains(&query) || a.content.to_lowercase().contains(&query))
        .collect();

    let active_artifact = filtered_artifacts.get(selected_tab()).cloned().or_else(|| filtered_artifacts.first().cloned());
    let (has_active, active_title, active_content, active_type) = match active_artifact {
        Some(art) => (true, art.title, art.content, art.artifact_type),
        None => (false, String::new(), String::new(), ArtifactType::MarkdownDoc),
    };

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-hidden flex flex-col justify-between select-text",
            // Header
            header { class: "px-8 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "flex items-center space-x-3",
                    span { class: "text-base font-bold text-white tracking-tight", "Live Artifact Studio" }
                    span { class: "text-xs font-mono text-emerald-400 bg-emerald-950/60 border border-emerald-800/80 px-2 py-0.5 rounded", "Real-Time Diff & Data Explorer" }
                }
                div { class: "flex items-center space-x-3",
                    input {
                        class: "bg-[#16171d] text-slate-300 text-xs rounded-lg px-3 py-1.5 outline-none border border-[#1e293b] w-48 placeholder-slate-500",
                        placeholder: "Filter artifacts...",
                        value: "{filter_query}",
                        oninput: move |e| filter_query.set(e.value().clone()),
                    }
                }
            }

            // Main Studio Layout: Sidebar list + Detail Viewer
            div { class: "flex-1 flex overflow-hidden",
                // Left Artifacts Drawer
                div { class: "w-72 bg-[#070b14] border-r border-[#1e293b] flex flex-col p-4 space-y-2 overflow-y-auto select-none shrink-0",
                    span { class: "text-[10px] font-bold text-slate-500 tracking-wider uppercase mb-1 px-1", "Generated Artifacts" }
                    {filtered_artifacts.iter().enumerate().map(|(idx, item)| {
                        let is_active = selected_tab() == idx;
                        let t = item.title.clone();
                        let sz = item.size_bytes;
                        let type_icon = match item.artifact_type {
                            ArtifactType::CodeDiff => "⚡",
                            ArtifactType::TableData => "📊",
                            ArtifactType::MarkdownDoc => "📄",
                            ArtifactType::JsonPayload => "📦",
                        };
                        rsx! {
                            div {
                                key: "{item.id}",
                                class: if is_active {
                                    "flex items-center justify-between px-3 py-2.5 rounded-lg bg-[#1f212a] text-white font-medium cursor-pointer border border-[#1e293b]"
                                } else {
                                    "flex items-center justify-between px-3 py-2.5 rounded-lg hover:bg-[#16171d] text-slate-400 cursor-pointer transition duration-150"
                                },
                                onclick: move |_| selected_tab.set(idx),
                                div { class: "flex items-center space-x-2.5 truncate",
                                    span { class: "text-xs", "{type_icon}" }
                                    span { class: "text-xs truncate", "{t}" }
                                }
                                span { class: "text-[10px] font-mono text-slate-500 shrink-0", "{sz} B" }
                            }
                        }
                    })}
                }

                // Right Detail & Sandbox Canvas
                div { class: "flex-1 bg-[#040711] flex flex-col justify-between overflow-hidden p-6",
                    if has_active {
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl flex-1 flex flex-col overflow-hidden shadow-2xl",
                            // Top Action Bar
                            div { class: "px-6 py-3 border-b border-[#1e293b] bg-[#070b14] flex items-center justify-between select-none",
                                span { class: "text-xs font-semibold text-slate-200 font-mono", "{active_title}" }
                                div { class: "flex items-center space-x-2",
                                    button {
                                        class: "px-3 py-1 bg-[#16171d] hover:bg-[#1f212a] border border-[#1e293b] text-slate-300 text-xs font-medium rounded transition flex items-center space-x-1.5",
                                        onclick: {
                                            let text = active_content.clone();
                                            move |_| {
                                                crate::api::copyText(&text);
                                            }
                                        },
                                        span { "📋" }
                                        span { "Copy" }
                                    }
                                }
                            }

                            // Interactive Content Pane
                            div { class: "p-6 flex-1 overflow-y-auto text-xs text-slate-200 font-mono whitespace-pre-wrap leading-relaxed",
                                match active_type {
                                    ArtifactType::CodeDiff => rsx! {
                                        div { class: "space-y-1 font-mono",
                                            for line in active_content.lines() {
                                                if line.starts_with('+') {
                                                    div { class: "bg-emerald-950/40 text-emerald-300 px-2 py-0.5 rounded font-mono", "{line}" }
                                                } else if line.starts_with('-') {
                                                    div { class: "bg-red-950/40 text-red-300 px-2 py-0.5 rounded font-mono", "{line}" }
                                                } else if line.starts_with('@') {
                                                    div { class: "text-cyan-400 font-bold py-1", "{line}" }
                                                } else {
                                                    div { class: "text-slate-300 px-2", "{line}" }
                                                }
                                            }
                                        }
                                    },
                                    ArtifactType::TableData => rsx! {
                                        div { class: "p-2 bg-[#070b14] border border-[#1e293b] rounded-lg overflow-x-auto",
                                            crate::components::markdown::MarkdownView { content: active_content.clone() }
                                        }
                                    },
                                    _ => rsx! {
                                        crate::components::markdown::MarkdownView { content: active_content.clone() }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "flex-1 flex items-center justify-center text-slate-600 italic select-none",
                            "Select an artifact from the sidebar to inspect."
                        }
                    }
                }
            }
        }
    }
}

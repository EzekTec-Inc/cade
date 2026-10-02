use crate::api_engine::{ApiClientEngine, ResourceState};
use crate::types::AppState;
use dioxus::prelude::*;

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
    let engine = use_context::<Memo<ApiClientEngine>>();
    let mut selected_tab = use_signal(|| 0usize);
    let mut filter_query = use_signal(String::new);
    let mut server_artifacts = use_signal(|| ResourceState::<Vec<serde_json::Value>>::Loading);

    // Fetch real artifacts from the server repository for the active agent
    let agent_id_opt = (state.selected_agent)().map(|a| a.id);
    use_effect(move || {
        let eng = engine();
        let agent_id = agent_id_opt.clone();
        spawn(async move {
            if let Some(id) = agent_id {
                let res = eng.fetch_artifacts(&id).await;
                server_artifacts.set(res);
            } else {
                server_artifacts.set(ResourceState::Ready(Vec::new()));
            }
        });
    });

    // Collate real server-backed artifacts and active session chat messages
    let mut detected_artifacts = Vec::<ArtifactItem>::new();

    // 1. Add server repository artifacts
    if let Some(items) = server_artifacts.read().value() {
        for item in items {
            let id = item["id"].as_str().unwrap_or("art").to_string();
            let kind = item["kind"].as_str().unwrap_or("document");
            let data_text = item["data_text"].as_str().unwrap_or("").to_string();
            let size_bytes = item["size_bytes"].as_u64().unwrap_or(data_text.len() as u64) as usize;

            let (artifact_type, title) = match kind {
                "diff" => (ArtifactType::CodeDiff, format!("Diff ({id})")),
                "table" | "dataset" => (ArtifactType::TableData, format!("Dataset ({id})")),
                "json" => (ArtifactType::JsonPayload, format!("JSON ({id})")),
                _ => (ArtifactType::MarkdownDoc, format!("Artifact ({id})")),
            };

            detected_artifacts.push(ArtifactItem {
                id,
                title,
                artifact_type,
                content: data_text,
                size_bytes,
            });
        }
    }

    // 2. Add inline artifacts detected from conversation turns
    let msgs = (state.messages)();
    for (i, m) in msgs.iter().enumerate() {
        let text = match &m.content {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        if text.contains("```diff") || (text.contains("--- ") && text.contains("+++ ")) {
            detected_artifacts.push(ArtifactItem {
                id: format!("session-diff-{i}"),
                title: format!("Code Patch #{}", i + 1),
                artifact_type: ArtifactType::CodeDiff,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.contains("```json") || (text.starts_with('{') && text.ends_with('}')) {
            detected_artifacts.push(ArtifactItem {
                id: format!("session-json-{i}"),
                title: format!("Structured Data #{}", i + 1),
                artifact_type: ArtifactType::JsonPayload,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.contains('|') && text.contains("---") {
            detected_artifacts.push(ArtifactItem {
                id: format!("session-table-{i}"),
                title: format!("Tabular Dataset #{}", i + 1),
                artifact_type: ArtifactType::TableData,
                content: text.clone(),
                size_bytes: text.len(),
            });
        } else if text.len() > 300 {
            detected_artifacts.push(ArtifactItem {
                id: format!("session-doc-{i}"),
                title: format!("Documentation Note #{}", i + 1),
                artifact_type: ArtifactType::MarkdownDoc,
                content: text.clone(),
                size_bytes: text.len(),
            });
        }
    }

    let query = filter_query().to_lowercase();
    let filtered_artifacts: Vec<ArtifactItem> = detected_artifacts
        .into_iter()
        .filter(|a| {
            query.is_empty()
                || a.title.to_lowercase().contains(&query)
                || a.content.to_lowercase().contains(&query)
        })
        .collect();

    let active_artifact = filtered_artifacts
        .get(selected_tab())
        .cloned()
        .or_else(|| filtered_artifacts.first().cloned());
    let (has_active, active_title, active_content, active_type) = match active_artifact {
        Some(art) => (true, art.title, art.content, art.artifact_type),
        None => (
            false,
            String::new(),
            String::new(),
            ArtifactType::MarkdownDoc,
        ),
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
                    if filtered_artifacts.is_empty() {
                        div { class: "p-4 text-center text-xs text-slate-500 font-mono", "(no artifacts)" }
                    } else {
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
                        div { class: "flex-1 flex flex-col items-center justify-center p-12 text-slate-400 font-sans select-none",
                            span { class: "text-3xl mb-3", "📦" }
                            p { class: "font-semibold text-slate-300 mb-1", "No artifacts generated yet" }
                            p { class: "text-xs text-slate-500 max-w-sm text-center", "Code diffs, structured tables, and reports produced during agent sessions will appear here in real time." }
                        }
                    }
                }
            }
        }
    }
}

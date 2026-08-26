//! Reactive Workflow & Pipeline Visualizer UI (PRD #99 / Issue #102).

use cade_api_types::WorkflowSummary;
use dioxus::prelude::*;

use crate::api_engine::{ApiClientEngine, ResourceState};

#[component]
pub fn WorkflowView() -> Element {
    let api_engine = use_context::<ApiClientEngine>();
    let mut workflows_res = use_signal(|| ResourceState::<Vec<WorkflowSummary>>::Loading);
    let mut trigger_output = use_signal(|| None::<String>);

    let engine_for_effect = api_engine.clone();
    use_effect(move || {
        let engine = engine_for_effect.clone();
        spawn(async move {
            let res = engine.fetch_workflows().await;
            workflows_res.set(res);
        });
    });

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                div {
                    h1 { class: "text-lg font-semibold text-white", "Automated Workflows & Pipelines" }
                    p { class: "text-xs text-gray-400 mt-0.5", "Multi-step agent pipelines, CI/CD validation, and automated workflows." }
                }
            }

            div { class: "p-10 max-w-6xl mx-auto space-y-8",
                match workflows_res() {
                    ResourceState::Loading => rsx! {
                        div { class: "text-center text-gray-400 py-12", "Loading workflows..." }
                    },
                    ResourceState::Error(err) => rsx! {
                        div { class: "bg-red-950/40 border border-red-800/50 rounded-xl p-6 text-red-300",
                            "Failed to load workflows: {err}"
                        }
                    },
                    ResourceState::Ready(list) => rsx! {
                        // Visual DAG Canvas Area
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-6 shadow-xl space-y-6",
                            div { class: "flex items-center justify-between border-b border-[#1e293b] pb-4 select-none",
                                div { class: "flex items-center space-x-2.5",
                                    span { class: "text-white font-bold text-sm", "Visual DAG Pipeline Visualizer" }
                                    span { class: "text-xs font-mono text-cyan-400 bg-cyan-950/60 border border-cyan-800/80 px-2 py-0.5 rounded", "Autonomous Subagent Nodes" }
                                }
                                span { class: "text-xs font-mono text-slate-400", "Topology: Sequential & Fan-Out" }
                            }

                            // Interactive DAG node graph
                            div { class: "flex items-center justify-between gap-4 overflow-x-auto py-6 px-2 select-none",
                                // Node 1: Scout
                                div { class: "w-56 bg-[#070b14] border border-[#1e293b] rounded-xl p-4 flex flex-col space-y-2 shadow-lg shrink-0",
                                    div { class: "flex items-center justify-between",
                                        span { class: "text-xs font-bold text-slate-100 font-mono", "1. Scout & Index" }
                                        span { class: "w-2 h-2 rounded-full bg-emerald-400" }
                                    }
                                    p { class: "text-[11px] text-slate-400", "Parse AST symbols & locate codebase seams." }
                                    div { class: "pt-2 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-slate-500",
                                        span { "claude-3-5-haiku" }
                                        span { "Done (310ms)" }
                                    }
                                }

                                span { class: "text-slate-600 font-bold text-lg shrink-0", "➔" }

                                // Node 2: Architect
                                div { class: "w-56 bg-[#070b14] border border-cyan-500/50 rounded-xl p-4 flex flex-col space-y-2 shadow-lg shrink-0 ring-1 ring-cyan-500/20",
                                    div { class: "flex items-center justify-between",
                                        span { class: "text-xs font-bold text-cyan-400 font-mono", "2. Refactor Code" }
                                        span { class: "w-2 h-2 rounded-full bg-cyan-400 animate-pulse" }
                                    }
                                    p { class: "text-[11px] text-slate-400", "Apply deep module mutations & AST edits." }
                                    div { class: "pt-2 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-cyan-500",
                                        span { "claude-3-5-sonnet" }
                                        span { "Executing..." }
                                    }
                                }

                                span { class: "text-slate-600 font-bold text-lg shrink-0", "➔" }

                                // Node 3: QA Tester
                                div { class: "w-56 bg-[#070b14] border border-[#1e293b] rounded-xl p-4 flex flex-col space-y-2 shadow-lg shrink-0 opacity-70",
                                    div { class: "flex items-center justify-between",
                                        span { class: "text-xs font-bold text-slate-300 font-mono", "3. Verify & Test" }
                                        span { class: "w-2 h-2 rounded-full bg-slate-600" }
                                    }
                                    p { class: "text-[11px] text-slate-400", "Execute cargo test & strict clippy checks." }
                                    div { class: "pt-2 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-slate-500",
                                        span { "worker-qa" }
                                        span { "Pending" }
                                    }
                                }
                            }
                        }

                        // Registered Workflows Grid
                        if list.is_empty() {
                            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center text-slate-400",
                                "No registered workflows found. Create workflow definitions in .cade/workflows/"
                            }
                        } else {
                            div { class: "grid grid-cols-1 md:grid-cols-2 gap-6",
                                for wf in list {
                                    div {
                                        key: "{wf.id}",
                                        class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-6 hover:border-slate-600 transition flex flex-col justify-between shadow-xl",
                                        div {
                                            div { class: "flex items-center justify-between mb-3",
                                                h3 { class: "text-white font-semibold text-base", "{wf.name}" }
                                                span { class: "text-xs px-2.5 py-0.5 rounded-full bg-blue-950 text-blue-400 border border-blue-800/50",
                                                    "{wf.steps_count} step(s)"
                                                }
                                            }
                                            p { class: "text-slate-400 text-xs mb-4", "{wf.description}" }
                                            if let Some(ref last) = wf.last_run {
                                                div { class: "text-xs text-slate-500 mb-4 flex items-center gap-2",
                                                    span { "Last run status: " }
                                                    span { class: "font-mono text-slate-300", "{last.status.as_str()}" }
                                                }
                                            }
                                        }
                                        div { class: "pt-4 border-t border-[#1e293b] flex items-center justify-between",
                                            button {
                                                class: "px-4 py-1.5 bg-sky-600 hover:bg-sky-500 text-white rounded-lg text-xs font-medium transition",
                                                onclick: {
                                                    let name = wf.name.clone();
                                                    let engine = api_engine.clone();
                                                    move |_| {
                                                        let name = name.clone();
                                                        let engine = engine.clone();
                                                        spawn(async move {
                                                            match engine.dispatch_workflow_run(&name, serde_json::json!({})).await {
                                                                Ok(run_id) => {
                                                                    trigger_output.set(Some(format!("Triggered workflow '{name}' -> Run ID: {run_id}")));
                                                                }
                                                                Err(e) => {
                                                                    trigger_output.set(Some(format!("Failed to trigger: {e}")));
                                                                }
                                                            }
                                                        });
                                                    }
                                                },
                                                "Run Pipeline"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(ref msg) = trigger_output() {
                    div { class: "bg-blue-950/40 border border-blue-800/50 rounded-xl p-4 text-blue-300 text-sm font-mono",
                        "{msg}"
                    }
                }
            }
        }
    }
}

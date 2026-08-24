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
                        if list.is_empty() {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center text-gray-400",
                                "No registered workflows found. Create workflow definitions in .cade/workflows/"
                            }
                        } else {
                            div { class: "grid grid-cols-1 md:grid-cols-2 gap-6",
                                for wf in list {
                                    div {
                                        key: "{wf.id}",
                                        class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 hover:border-gray-600 transition flex flex-col justify-between",
                                        div {
                                            div { class: "flex items-center justify-between mb-3",
                                                h3 { class: "text-white font-semibold text-base", "{wf.name}" }
                                                span { class: "text-xs px-2.5 py-0.5 rounded-full bg-blue-900/40 text-blue-300 border border-blue-800/50",
                                                    "{wf.steps_count} step(s)"
                                                }
                                            }
                                            p { class: "text-gray-400 text-sm mb-4", "{wf.description}" }
                                            if let Some(ref last) = wf.last_run {
                                                div { class: "text-xs text-gray-500 mb-4 flex items-center gap-2",
                                                    span { "Last run status: " }
                                                    span { class: "font-mono text-gray-300", "{last.status.as_str()}" }
                                                }
                                            }
                                        }
                                        div { class: "pt-4 border-t border-[#272833]/60 flex items-center justify-between",
                                            button {
                                                class: "px-4 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-medium transition",
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

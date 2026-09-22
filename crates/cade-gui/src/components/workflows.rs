//! Reactive Workflow & Pipeline Visualizer UI (PRD #99 / Issue #102).

use cade_api_types::{WorkflowStatus, WorkflowSummary};
use dioxus::prelude::*;

use crate::api_engine::ResourceState;
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn WorkflowView() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let workflows_res = use_signal(|| ResourceState::<Vec<WorkflowSummary>>::Loading);
    let selected_id = use_signal(String::new);
    let trigger_output = use_signal(|| None::<String>);
    let is_running = use_signal(|| false);

    let fetch_all = move || {
        let engine = crate::api_engine::ApiClientEngine::new(client);
        let mut w_res = workflows_res;
        let mut sel_id = selected_id;
        spawn(async move {
            let res = engine.fetch_workflows().await;
            if let ResourceState::Ready(ref list) = res
                && sel_id().is_empty()
                && !list.is_empty()
            {
                sel_id.set(list[0].id.clone());
            }
            w_res.set(res);
        });
    };

    use_effect(move || {
        fetch_all();
    });

    let run_workflow = move |wf_name: String| {
        let engine = crate::api_engine::ApiClientEngine::new(client);
        let st = state;
        let mut running = is_running;
        let mut out = trigger_output;
        let mut w_res = workflows_res;
        running.set(true);
        out.set(Some(format!("Dispatching pipeline '{wf_name}'...")));
        spawn(async move {
            match engine
                .dispatch_workflow_run(&wf_name, serde_json::json!({}))
                .await
            {
                Ok(run_id) => {
                    add_toast(
                        &st,
                        ToastLevel::Success,
                        "Pipeline Dispatched",
                        format!("Run ID: {run_id}"),
                    );
                    out.set(Some(format!(
                        "✓ Dispatched '{wf_name}' -> Active Run ID: {run_id}"
                    )));
                    let updated = engine.fetch_workflows().await;
                    w_res.set(updated);
                }
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Pipeline Failed", e.clone());
                    out.set(Some(format!("✕ Failed to dispatch '{wf_name}': {e}")));
                }
            }
            running.set(false);
        });
    };

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text flex flex-col",
            header { class: "px-10 py-5 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "space-y-1",
                    h1 { class: "text-lg font-bold text-slate-100 tracking-tight", "Automated Workflows & DAG Pipelines" }
                    p { class: "text-xs text-slate-400", "Multi-step autonomous agent pipelines, CI/CD validation, and dependency graph execution." }
                }
                button {
                    class: "text-xs bg-[#16171d] hover:bg-[#1f212a] text-slate-300 border border-[#1e293b] rounded-lg px-3 py-1.5 font-medium transition flex items-center space-x-1.5 cursor-pointer",
                    onclick: move |_| fetch_all(),
                    span { "↻" }
                    span { "Refresh" }
                }
            }

            div { class: "p-8 max-w-7xl mx-auto w-full space-y-6 flex-1",
                match workflows_res() {
                    ResourceState::Loading => rsx! {
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-12 text-center",
                            p { class: "text-slate-400 text-xs animate-pulse font-mono", "Loading workflow pipelines and DAG definitions from server..." }
                        }
                    },
                    ResourceState::Error(err) => rsx! {
                        div { class: "bg-red-950/30 border border-red-800/40 rounded-xl p-6 text-red-300 space-y-2",
                            div { class: "font-bold text-sm", "Failed to load workflows" }
                            div { class: "text-xs font-mono text-red-400", "{err}" }
                        }
                    },
                    ResourceState::Ready(list) => {
                        let current_selected = list.iter().find(|w| w.id == selected_id()).or_else(|| list.first());
                        rsx! {
                            if let Some(selected_wf) = current_selected {
                                {
                                    let wf_name = selected_wf.name.clone();
                                    let wf_steps = selected_wf.steps.clone();
                                    let is_active_running = is_running();
                                    let steps_count = wf_steps.len();

                                    rsx! {
                                        // Visual DAG Canvas Area
                                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-6 shadow-2xl space-y-5",
                                            div { class: "flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 border-b border-[#1e293b]/70 pb-4 select-none",
                                                div { class: "space-y-1",
                                                    div { class: "flex items-center space-x-3",
                                                        span { class: "text-slate-100 font-bold text-base font-mono", "{selected_wf.name}" }
                                                        span { class: "text-xs font-mono text-cyan-400 bg-cyan-950/60 border border-cyan-800/80 px-2 py-0.5 rounded",
                                                            "{steps_count} DAG Step(s)"
                                                        }
                                                        if let Some(ref last) = selected_wf.last_run {
                                                            {
                                                                let status_color = match last.status {
                                                                    WorkflowStatus::Succeeded => "text-emerald-400 bg-emerald-950/50 border-emerald-800/60",
                                                                    WorkflowStatus::Failed => "text-red-400 bg-red-950/50 border-red-800/60",
                                                                    WorkflowStatus::Running => "text-cyan-400 bg-cyan-950/50 border-cyan-800/60 animate-pulse",
                                                                    _ => "text-slate-400 bg-slate-900 border-slate-700",
                                                                };
                                                                rsx! {
                                                                    span { class: "text-[11px] font-mono px-2 py-0.5 rounded border {status_color}",
                                                                        "Last Run: {last.status.as_str()}"
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    p { class: "text-xs text-slate-400", "{selected_wf.description}" }
                                                }

                                                button {
                                                    class: "text-xs bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg px-4 py-2 font-semibold transition flex items-center space-x-2 shadow-lg disabled:opacity-50 cursor-pointer",
                                                    disabled: is_active_running,
                                                    onclick: {
                                                        let name = wf_name.clone();
                                                        move |_| run_workflow(name.clone())
                                                    },
                                                    span { if is_active_running { "⠋" } else { "▶" } }
                                                    span { if is_active_running { "Executing Pipeline..." } else { "Run Pipeline" } }
                                                }
                                            }

                                            // Real Dynamic DAG Nodes
                                            if wf_steps.is_empty() {
                                                div { class: "p-8 text-center text-slate-500 text-xs font-mono",
                                                    "No steps defined in this workflow configuration."
                                                }
                                            } else {
                                                div { class: "flex items-center gap-3 overflow-x-auto py-4 px-1 select-none",
                                                    {wf_steps.into_iter().enumerate().map(|(idx, step)| {
                                                        let is_last = idx + 1 == steps_count;
                                                        let agent_label = step.agent.clone().unwrap_or_else(|| "worker".to_string());
                                                        let step_name = step.name.clone();
                                                        let prompt_text = step.prompt.clone();
                                                        let deps = step.depends_on.clone();

                                                        rsx! {
                                                            // Step Node Card
                                                            div {
                                                                key: "{step_name}",
                                                                class: "w-64 bg-[#070b14] border border-[#1e293b] hover:border-cyan-500/50 rounded-xl p-4 flex flex-col justify-between space-y-3 shadow-lg shrink-0 transition",
                                                                div { class: "space-y-1.5",
                                                                    div { class: "flex items-center justify-between",
                                                                        span { class: "text-xs font-bold text-slate-100 font-mono",
                                                                            "{idx + 1}. {step_name}"
                                                                        }
                                                                        span { class: "w-2 h-2 rounded-full bg-cyan-400" }
                                                                    }
                                                                    p { class: "text-[11px] text-slate-400 line-clamp-2 leading-relaxed",
                                                                        "{prompt_text}"
                                                                    }
                                                                }

                                                                div { class: "pt-2 border-t border-[#1e293b]/60 flex flex-col space-y-1.5 text-[10px] font-mono",
                                                                    div { class: "flex items-center justify-between text-slate-400",
                                                                        span { class: "text-purple-400 bg-purple-950/40 border border-purple-800/50 px-1.5 py-0.5 rounded",
                                                                            "@{agent_label}"
                                                                        }
                                                                        if !deps.is_empty() {
                                                                            span { class: "text-slate-500 truncate max-w-[120px]",
                                                                                "deps: {deps.join(\", \")}"
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            // Directed Dependency Edge Arrow
                                                            if !is_last {
                                                                div { class: "flex flex-col items-center justify-center px-1 shrink-0 text-slate-600 font-bold text-lg",
                                                                    span { "➔" }
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

                            // Registered Workflows Grid
                            div { class: "space-y-4 pt-2",
                                h2 { class: "text-sm font-bold text-slate-200 uppercase tracking-wider", "Available Pipeline Workflows" }
                                if list.is_empty() {
                                    div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center text-slate-400",
                                        "No registered workflows found. Configure pipeline definitions in .cade/workflows/"
                                    }
                                } else {
                                    div { class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5",
                                        for wf in list {
                                            {
                                                let wf_id = wf.id.clone();
                                                let wf_name = wf.name.clone();
                                                let is_selected = wf_id == selected_id();
                                                let is_active_running = is_running();

                                                rsx! {
                                                    div {
                                                        key: "{wf_id}",
                                                        class: if is_selected {
                                                            "bg-[#090d16] border border-cyan-500/60 ring-1 ring-cyan-500/30 rounded-xl p-5 transition flex flex-col justify-between shadow-xl cursor-pointer"
                                                        } else {
                                                            "bg-[#090d16] border border-[#1e293b] hover:border-slate-600 rounded-xl p-5 transition flex flex-col justify-between shadow-xl cursor-pointer"
                                                        },
                                                        onclick: {
                                                            let id = wf_id.clone();
                                                            let mut sel = selected_id;
                                                            move |_| sel.set(id.clone())
                                                        },
                                                        div { class: "space-y-2",
                                                            div { class: "flex items-center justify-between",
                                                                h3 { class: "text-slate-100 font-semibold text-sm font-mono truncate", "{wf.name}" }
                                                                span { class: "text-[11px] px-2 py-0.5 rounded-full bg-indigo-950/60 text-indigo-400 border border-indigo-800/50 font-mono shrink-0",
                                                                    "{wf.steps_count} step(s)"
                                                                }
                                                            }
                                                            p { class: "text-slate-400 text-xs line-clamp-2 leading-relaxed", "{wf.description}" }
                                                            if let Some(ref last) = wf.last_run {
                                                                div { class: "text-[11px] text-slate-500 flex items-center space-x-2 pt-1 font-mono",
                                                                    span { "Status:" }
                                                                    span { class: "text-slate-300 font-semibold", "{last.status.as_str()}" }
                                                                }
                                                            }
                                                        }

                                                        div { class: "pt-4 mt-3 border-t border-[#1e293b]/60 flex items-center justify-between select-none",
                                                            span { class: if is_selected { "text-cyan-400 text-xs font-semibold" } else { "text-slate-500 text-xs hover:text-slate-300" },
                                                                if is_selected { "● Viewing DAG" } else { "Click to View DAG" }
                                                            }
                                                            button {
                                                                class: "px-3 py-1 bg-sky-600 hover:bg-sky-500 text-white rounded-lg text-xs font-medium transition cursor-pointer shadow-sm disabled:opacity-50",
                                                                disabled: is_active_running,
                                                                onclick: {
                                                                    let name = wf_name.clone();
                                                                    move |e: MouseEvent| {
                                                                        e.stop_propagation();
                                                                        run_workflow(name.clone());
                                                                    }
                                                                },
                                                                "Run"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(ref msg) = trigger_output() {
                    div { class: "bg-[#09152b] border border-cyan-500/40 rounded-xl p-4 text-cyan-300 text-xs font-mono shadow-xl flex items-center justify-between",
                        span { "{msg}" }
                        button {
                            class: "text-slate-400 hover:text-slate-200 text-xs px-2 py-0.5 rounded hover:bg-white/5 cursor-pointer",
                            onclick: {
                                let mut out = trigger_output;
                                move |_| out.set(None)
                            },
                            "✕"
                        }
                    }
                }
            }
        }
    }
}

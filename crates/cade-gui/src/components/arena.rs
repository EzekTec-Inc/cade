//! Multi-Model Arena Matrix & Parallel Stream Multiplexer (PRD #128 / Issue #129).

use dioxus::prelude::*;
use crate::api_engine::ApiClientEngine;

#[derive(Clone, PartialEq)]
pub struct ArenaLaneState {
    pub id: usize,
    pub agent_id: String,
    pub agent_name: String,
    pub model: String,
    pub content: String,
    pub is_streaming: bool,
    pub latency_ms: u64,
    pub token_count: usize,
    pub status: String,
}

#[component]
pub fn ArenaView() -> Element {
    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let engine = use_context::<ApiClientEngine>();

    let agents_list = use_signal(Vec::<cade_api_types::AgentInfo>::new);
    let mut prompt = use_signal(String::new);
    let mut is_running_all = use_signal(|| false);
    let mut show_diff = use_signal(|| false);

    let mut lanes = use_signal(|| vec![
        ArenaLaneState {
            id: 1,
            agent_id: String::new(),
            agent_name: "Model Lane A".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            content: String::new(),
            is_streaming: false,
            latency_ms: 0,
            token_count: 0,
            status: "Idle".to_string(),
        },
        ArenaLaneState {
            id: 2,
            agent_id: String::new(),
            agent_name: "Model Lane B".to_string(),
            model: "gpt-4o".to_string(),
            content: String::new(),
            is_streaming: false,
            latency_ms: 0,
            token_count: 0,
            status: "Idle".to_string(),
        },
    ]);

    // Populate initial agent list
    let eng_effect = engine.clone();
    use_effect(move || {
        let eng = eng_effect.clone();
        let mut ags = agents_list;
        let mut lns = lanes;
        spawn(async move {
            if let crate::api_engine::ResourceState::Ready(list) = eng.fetch_agents().await
                && !list.is_empty()
            {
                ags.set(list.clone());
                let mut current = lns();
                if current.len() >= 2 && list.len() >= 2 {
                    current[0].agent_id = list[0].id.clone();
                    current[0].agent_name = list[0].name.clone();
                    current[0].model = list[0].model.clone().unwrap_or_else(|| "claude-3-5-sonnet".to_string());
                    
                    current[1].agent_id = list[1].id.clone();
                    current[1].agent_name = list[1].name.clone();
                    current[1].model = list[1].model.clone().unwrap_or_else(|| "gpt-4o".to_string());
                    lns.set(current);
                } else if !current.is_empty() {
                    current[0].agent_id = list[0].id.clone();
                    current[0].agent_name = list[0].name.clone();
                    lns.set(current);
                }
            }
        });
    });

    let mut add_lane = move || {
        let mut cur = lanes();
        if cur.len() < 4 {
            let next_id = cur.len() + 1;
            let available_agent = agents_list().into_iter().nth(cur.len()).or_else(|| agents_list().first().cloned());
            let (aid, aname, amodel) = match available_agent {
                Some(a) => (a.id, a.name, a.model.unwrap_or_else(|| "default".to_string())),
                None => (String::new(), format!("Model Lane {next_id}"), "default".to_string()),
            };
            cur.push(ArenaLaneState {
                id: next_id,
                agent_id: aid,
                agent_name: aname,
                model: amodel,
                content: String::new(),
                is_streaming: false,
                latency_ms: 0,
                token_count: 0,
                status: "Idle".to_string(),
            });
            lanes.set(cur);
        }
    };

    let mut remove_lane = move |id: usize| {
        let mut cur = lanes();
        if cur.len() > 2 {
            cur.retain(|l| l.id != id);
            lanes.set(cur);
        }
    };

    let mut run_arena = move || {
        let p = prompt().trim().to_string();
        if p.is_empty() || is_running_all() {
            return;
        }
        is_running_all.set(true);

        let active_lanes = lanes();
        let api = client();
        let mut lns_sig = lanes;
        let mut running_sig = is_running_all;

        // Reset lane contents
        let mut init_lanes = active_lanes.clone();
        for l in &mut init_lanes {
            l.content = String::new();
            l.is_streaming = true;
            l.status = "Streaming...".to_string();
            l.latency_ms = 0;
            l.token_count = 0;
        }
        lns_sig.set(init_lanes);

        for lane in active_lanes {
            let api_client = api.clone();
            let aid = lane.agent_id.clone();
            let lane_id = lane.id;
            let prompt_text = p.clone();

            spawn(async move {
                let start = js_sys::Date::now();
                if aid.is_empty() {
                    let mut current = lns_sig();
                    if let Some(idx) = current.iter().position(|l| l.id == lane_id) {
                        current[idx].is_streaming = false;
                        current[idx].status = "No agent selected".to_string();
                    }
                    lns_sig.set(current);
                    return;
                }

                let stream_res = api_client.stream_messages(
                    &aid,
                    &prompt_text,
                    None,
                    None,
                    move |event: cade_api_types::StreamEvent| {
                        if let Some(delta) = event.content() {
                            let mut current = lns_sig();
                            if let Some(idx) = current.iter().position(|l| l.id == lane_id) {
                                current[idx].content.push_str(delta);
                                current[idx].token_count += delta.len() / 4 + 1;
                                current[idx].latency_ms = (js_sys::Date::now() - start) as u64;
                            }
                            lns_sig.set(current);
                        }
                    },
                ).await;

                let end = js_sys::Date::now();
                let mut current = lns_sig();
                if let Some(idx) = current.iter().position(|l| l.id == lane_id) {
                    current[idx].is_streaming = false;
                    current[idx].latency_ms = (end - start) as u64;
                    current[idx].status = match stream_res {
                        Ok(_) => "Completed".to_string(),
                        Err(e) => format!("Error: {e}"),
                    };
                }
                lns_sig.set(current);
            });
        }
        running_sig.set(false);
    };

    let active_lanes = lanes();
    let num_lanes = active_lanes.len();
    let grid_cols = match num_lanes {
        2 => "grid-cols-1 md:grid-cols-2",
        3 => "grid-cols-1 md:grid-cols-3",
        _ => "grid-cols-1 md:grid-cols-4",
    };

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto flex flex-col justify-between select-text",
            // Header
            header { class: "px-8 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "flex items-center space-x-3",
                    span { class: "text-base font-bold text-white tracking-tight", "Multi-Model Arena Matrix" }
                    span { class: "text-xs font-mono text-cyan-400 bg-cyan-950/60 border border-cyan-800/80 px-2 py-0.5 rounded", "Concurrent Streaming" }
                }
                div { class: "flex items-center space-x-3",
                    if num_lanes < 4 {
                        button {
                            class: "px-3 py-1.5 bg-[#16171d] hover:bg-[#1f212a] border border-[#1e293b] text-slate-300 text-xs font-medium rounded-lg transition",
                            onclick: move |_| add_lane(),
                            "+ Add Model Lane"
                        }
                    }
                    button {
                        class: if show_diff() { "px-3 py-1.5 bg-purple-600 text-white text-xs font-medium rounded-lg transition" } else { "px-3 py-1.5 bg-[#16171d] hover:bg-[#1f212a] border border-[#1e293b] text-slate-300 text-xs font-medium rounded-lg transition" },
                        onclick: move |_| show_diff.set(!show_diff()),
                        if show_diff() { "Diff Mode: Active" } else { "Toggle Diff View" }
                    }
                }
            }

            // Arena Lanes View
            div { class: "p-6 flex-1 overflow-y-auto",
                div { class: "grid {grid_cols} gap-4 h-full min-h-[480px]",
                    {active_lanes.iter().map(|lane| {
                        let l_id = lane.id;
                        let l_name = lane.agent_name.clone();
                        let l_model = lane.model.clone();
                        let l_content = lane.content.clone();
                        let l_is_stream = lane.is_streaming;
                        let l_status = lane.status.clone();
                        let l_lat = lane.latency_ms;
                        let l_tok = lane.token_count;
                        let can_remove = num_lanes > 2;

                        rsx! {
                            div {
                                key: "{l_id}",
                                class: "bg-[#090d16] border border-[#1e293b] rounded-xl flex flex-col justify-between overflow-hidden shadow-xl",
                                // Lane Header
                                div { class: "px-4 py-3 border-b border-[#1e293b] bg-[#070b14] flex items-center justify-between select-none",
                                    div { class: "flex flex-col min-w-0",
                                        span { class: "text-slate-100 font-semibold text-xs truncate", "{l_name}" }
                                        span { class: "text-[10px] font-mono text-cyan-400 truncate", "{l_model}" }
                                    }
                                    div { class: "flex items-center space-x-2",
                                        span { class: "text-[10px] font-mono text-slate-400 bg-[#16171d] px-2 py-0.5 rounded border border-[#1e293b]", "{l_lat} ms" }
                                        if can_remove {
                                            button {
                                                class: "text-slate-500 hover:text-red-400 text-xs p-1",
                                                title: "Remove Lane",
                                                onclick: move |_| remove_lane(l_id),
                                                "✖"
                                            }
                                        }
                                    }
                                }

                                // Lane Output Stream
                                div { class: "p-4 flex-1 overflow-y-auto text-xs text-slate-200 font-mono whitespace-pre-wrap leading-relaxed min-h-[300px] max-h-[500px]",
                                    if l_content.is_empty() {
                                        div { class: "h-full flex items-center justify-center text-slate-600 italic select-none",
                                            if l_is_stream { "Connecting to stream..." } else { "Awaiting prompt submission..." }
                                        }
                                    } else {
                                        "{l_content}"
                                    }
                                }

                                // Lane Footer Telemetry
                                div { class: "px-4 py-2 border-t border-[#1e293b] bg-[#070b14] flex items-center justify-between text-[10px] font-mono text-slate-400 select-none",
                                    div { class: "flex items-center space-x-1.5",
                                        span { class: if l_is_stream { "w-2 h-2 rounded-full bg-cyan-400 animate-pulse" } else { "w-2 h-2 rounded-full bg-slate-600" } }
                                        span { "{l_status}" }
                                    }
                                    span { "~{l_tok} tokens" }
                                }
                            }
                        }
                    })}
                }
            }

            // Bottom Prompt Input Bar
            div { class: "p-6 bg-[#040711] border-t border-[#1e293b]/70",
                div { class: "relative border border-[#1e293b] bg-[#090d16] rounded-xl p-3 flex items-center justify-between space-x-4",
                    textarea {
                        class: "bg-transparent text-gray-200 placeholder-gray-500 outline-none w-full text-xs resize-none h-14 font-mono",
                        placeholder: "Type comparison prompt to stream concurrently across all Arena lanes...",
                        value: "{prompt}",
                        oninput: move |e| prompt.set(e.value().clone()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !e.modifiers().shift() {
                                run_arena();
                            }
                        }
                    }
                    button {
                        class: "px-5 py-2.5 bg-gradient-to-r from-sky-500 to-indigo-600 hover:from-sky-400 hover:to-indigo-500 text-white font-medium text-xs rounded-lg transition duration-150 shrink-0 shadow-lg select-none",
                        onclick: move |_| run_arena(),
                        if is_running_all() { "Streaming..." } else { "Stream Arena ⚡" }
                    }
                }
            }
        }
    }
}

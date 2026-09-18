//! Real-Time Swarm Topology & Supervisory Hierarchy Tree (PRD #128 / Issue #133).

use cade_api_types::SwarmTopologyResponse;
use dioxus::prelude::*;

#[component]
pub fn SwarmView() -> Element {
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let topology = use_signal(|| None::<SwarmTopologyResponse>);
    let selected_team_id = use_signal(|| "default".to_string());
    let is_loading = use_signal(|| true);
    let error_msg = use_signal(|| None::<String>);

    let fetch_topology = move || {
        let api = client();
        let mut topo = topology;
        let mut loading = is_loading;
        let mut err = error_msg;
        let mut sel_team = selected_team_id;
        loading.set(true);
        err.set(None);
        spawn(async move {
            match api.list_swarm_topology().await {
                Ok(data) => {
                    if sel_team().is_empty() && !data.teams.is_empty() {
                        sel_team.set(data.teams[0].id.clone());
                    }
                    topo.set(Some(data));
                }
                Err(e) => {
                    err.set(Some(e));
                }
            }
            loading.set(false);
        });
    };

    use_effect(move || {
        fetch_topology();
    });

    let current_topo = topology();
    let current_team_id = selected_team_id();
    let selected_team = current_topo.as_ref().and_then(|topo| {
        topo.teams
            .iter()
            .find(|t| t.id == current_team_id)
            .or_else(|| topo.teams.first())
            .cloned()
    });

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto flex flex-col justify-between select-text",
            // Header
            header { class: "px-8 py-5 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "space-y-1",
                    div { class: "flex items-center space-x-3",
                        span { class: "text-base font-bold text-white tracking-tight", "Swarm Topology & Supervisory Tree" }
                        span { class: "text-xs font-mono text-purple-400 bg-purple-950/60 border border-purple-800/80 px-2 py-0.5 rounded", "Multi-Agent Swarm Hierarchy" }
                    }
                    p { class: "text-xs text-slate-400", "Live autonomous subagent trees, supervisory delegation hierarchies, and team topologies." }
                }
                div { class: "flex items-center space-x-3",
                    if let Some(ref data) = current_topo {
                        span { class: "text-xs font-mono text-slate-400 bg-[#141720] border border-[#1e293b] rounded-lg px-3 py-1.5",
                            "Active Nodes: {data.total_nodes}"
                        }
                    }
                    button {
                        class: "text-xs bg-[#16171d] hover:bg-[#1f212a] text-slate-300 border border-[#1e293b] rounded-lg px-3 py-1.5 font-medium transition flex items-center space-x-1.5 cursor-pointer",
                        onclick: move |_| fetch_topology(),
                        span { "↻" }
                        span { "Refresh Topology" }
                    }
                }
            }

            // Main Content Area
            div { class: "p-8 max-w-6xl mx-auto w-full space-y-8 flex-1",
                if is_loading() && current_topo.is_none() {
                    div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-12 text-center",
                        p { class: "text-slate-400 text-xs animate-pulse font-mono", "Discovering active agent teams and subagent topologies..." }
                    }
                } else if let Some(ref err) = error_msg() {
                    div { class: "bg-red-950/30 border border-red-800/40 rounded-xl p-6 text-red-300 space-y-2",
                        div { class: "font-bold text-sm", "Failed to load Swarm Topology" }
                        div { class: "text-xs font-mono text-red-400", "{err}" }
                    }
                } else if let Some(ref topo) = current_topo {
                    div { class: "space-y-8",
                        // Team Selection Switcher
                        if !topo.teams.is_empty() {
                            div { class: "flex items-center space-x-3 overflow-x-auto pb-2 select-none",
                                span { class: "text-xs font-mono text-slate-500 uppercase tracking-wider", "Team:" }
                                for team in &topo.teams {
                                    button {
                                        key: "{team.id}",
                                        class: if team.id == current_team_id {
                                            "px-3.5 py-1.5 rounded-lg bg-purple-500/20 text-purple-300 border border-purple-500/40 font-semibold text-xs font-mono transition flex items-center space-x-2 cursor-pointer"
                                        } else {
                                            "px-3.5 py-1.5 rounded-lg bg-[#090d16] hover:bg-[#141720] text-slate-400 hover:text-slate-200 border border-[#1e293b] font-medium text-xs font-mono transition flex items-center space-x-2 cursor-pointer"
                                        },
                                        onclick: {
                                            let t_id = team.id.clone();
                                            let mut sel = selected_team_id;
                                            move |_| sel.set(t_id.clone())
                                        },
                                        span { "{team.name}" }
                                        span { class: "text-[10px] bg-slate-800 px-1.5 py-0.2 rounded-full", "{team.members.len()}" }
                                    }
                                }
                            }
                        }

                        // Display Selected Team Hierarchy Tree
                        if let Some(ref team) = selected_team {
                            div { class: "space-y-6",
                                // Team Summary Banner
                                div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 flex flex-col md:flex-row items-start md:items-center justify-between gap-4 shadow-xl select-none",
                                    div { class: "space-y-1",
                                        div { class: "flex items-center space-x-2.5",
                                            span { class: "text-slate-100 font-bold text-sm font-mono", "{team.name}" }
                                            span { class: "text-[10px] font-mono text-cyan-400 bg-cyan-950/60 border border-cyan-800/80 px-2 py-0.5 rounded",
                                                "Mode: {team.mode}"
                                            }
                                            span { class: "text-[10px] font-mono text-purple-400 bg-purple-950/60 border border-purple-800/80 px-2 py-0.5 rounded",
                                                "Scope: {team.scope}"
                                            }
                                        }
                                        p { class: "text-xs text-slate-400", "{team.description}" }
                                    }
                                    div { class: "flex items-center space-x-3 text-xs font-mono text-slate-400",
                                        span { "Max Iterations: {team.max_iterations}" }
                                    }
                                }

                                // Supervisory Tree Visualization
                                div { class: "flex flex-col items-center space-y-6 pt-2 select-none",
                                    // Lead Coordinator Node
                                    div { class: "w-80 bg-[#090d16] border border-purple-500/60 shadow-[0_0_25px_rgba(168,85,247,0.18)] rounded-xl p-5 flex flex-col space-y-3 transition",
                                        div { class: "flex items-center justify-between",
                                            div { class: "flex items-center space-x-2",
                                                span { "👑" }
                                                span { class: "text-slate-100 font-bold text-xs font-mono", "Lead Supervisor" }
                                            }
                                            div { class: "flex items-center space-x-1.5",
                                                span { class: "w-2 h-2 rounded-full bg-emerald-400 animate-pulse" }
                                                span { class: "text-[10px] font-mono text-emerald-400 font-semibold", "Active" }
                                            }
                                        }
                                        p { class: "text-slate-400 text-[11px] leading-relaxed",
                                            "Primary coordinator delegating tasks, enforcing verification gates, and orchestrating member execution."
                                        }
                                        div { class: "pt-2 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-slate-400",
                                            span { class: "text-purple-300", "{team.leader_model.as_deref().unwrap_or(\"anthropic/claude-sonnet-4-5\")}" }
                                            span { "Role: Coordinator" }
                                        }
                                    }

                                    // Branch Connector Line
                                    div { class: "w-0.5 h-8 bg-gradient-to-b from-purple-500 to-indigo-500" }

                                    // Team Members Grid (The actual discovered members!)
                                    div { class: "grid grid-cols-1 md:grid-cols-3 gap-6 w-full",
                                        for member in &team.members {
                                            div {
                                                key: "{member.id}",
                                                class: "bg-[#090d16] border border-[#1e293b] hover:border-slate-600 rounded-xl p-5 shadow-xl flex flex-col justify-between space-y-3 transition duration-150",
                                                div { class: "space-y-2",
                                                    div { class: "flex items-center justify-between",
                                                        div { class: "flex items-center space-x-2",
                                                            span { class: "text-slate-100 font-bold text-xs font-mono", "{member.name}" }
                                                        }
                                                        div { class: "flex items-center space-x-1.5",
                                                            span { class: "w-2 h-2 rounded-full bg-cyan-400" }
                                                            span { class: "text-[10px] font-mono text-slate-400", "{member.status}" }
                                                        }
                                                    }
                                                    if let Some(ref role) = member.role {
                                                        span { class: "text-[10px] font-mono text-purple-400 bg-purple-950/40 border border-purple-800/50 px-2 py-0.5 rounded inline-block",
                                                            "{role}"
                                                        }
                                                    }
                                                    p { class: "text-slate-400 text-[11px] leading-relaxed line-clamp-3", "{member.description}" }
                                                }
                                                div { class: "pt-3 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-slate-500",
                                                    span { class: "text-slate-400", "{member.model.as_deref().unwrap_or(\"Inherited\")}" }
                                                    span { class: "text-slate-500", "Tools: {member.tools}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Standalone Subagents Swarm
                        if !topo.standalone_subagents.is_empty() {
                            div { class: "space-y-4 pt-4 border-t border-[#1e293b]/70 select-none",
                                div { class: "flex items-center justify-between",
                                    h2 { class: "text-xs font-bold text-slate-300 uppercase tracking-wider font-mono", "Specialist Autonomous Subagents" }
                                    span { class: "text-xs font-mono text-slate-500", "{topo.standalone_subagents.len()} discovered subagent(s)" }
                                }
                                div { class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4",
                                    for sub in &topo.standalone_subagents {
                                        div {
                                            key: "{sub.id}",
                                            class: "bg-[#070b14] border border-[#1e293b] rounded-xl p-4 space-y-2 flex flex-col justify-between shadow-lg",
                                            div { class: "space-y-1.5",
                                                div { class: "flex items-center justify-between",
                                                    span { class: "text-xs font-bold text-slate-200 font-mono", "{sub.name}" }
                                                    span { class: "text-[9px] font-mono px-1.5 py-0.5 rounded border border-cyan-800/60 bg-cyan-950/40 text-cyan-400", "SUBAGENT" }
                                                }
                                                p { class: "text-[11px] text-slate-400 line-clamp-2 leading-relaxed", "{sub.description}" }
                                            }
                                            div { class: "pt-2 border-t border-[#1e293b]/60 flex items-center justify-between text-[10px] font-mono text-slate-500",
                                                span { "{sub.model.as_deref().unwrap_or(\"auto\")}" }
                                                span { "{sub.status}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Intercom Protocol & Coordination Status
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 shadow-xl space-y-3 select-none",
                            div { class: "flex items-center justify-between border-b border-[#1e293b] pb-2.5",
                                div { class: "flex items-center space-x-2.5",
                                    span { class: "text-slate-100 font-bold text-xs font-mono", "Inter-Agent Intercom Protocol (ADR-0015 / ADR-0021)" }
                                    span { class: "text-[10px] font-mono text-emerald-400 bg-emerald-950/60 border border-emerald-800/80 px-2 py-0.5 rounded", "Ready" }
                                }
                                span { class: "text-[11px] font-mono text-slate-500", "Channel: Non-blocking IPC" }
                            }
                            div { class: "grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs font-mono text-slate-400",
                                div { class: "bg-[#070b14] border border-[#1e293b]/60 rounded-lg p-3 space-y-1",
                                span { class: "text-[10px] text-slate-500 uppercase tracking-wider", "Coordination Mode" }
                                div { class: "text-slate-200 font-bold", "Hierarchical & Sequential" }
                            }
                            div { class: "bg-[#070b14] border border-[#1e293b]/60 rounded-lg p-3 space-y-1",
                                span { class: "text-[10px] text-slate-500 uppercase tracking-wider", "Sandboxing" }
                                div { class: "text-slate-200 font-bold", "Isolated Git Worktrees" }
                            }
                            div { class: "bg-[#070b14] border border-[#1e293b]/60 rounded-lg p-3 space-y-1",
                                span { class: "text-[10px] text-slate-500 uppercase tracking-wider", "Verification Gates" }
                                div { class: "text-emerald-400 font-bold", "Required Tests Enforced" }
                            }
                        }
                    }
                }
            }
        }
    }
}
}

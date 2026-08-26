//! Real-Time Swarm Topology & Supervisory Hierarchy Tree (PRD #128 / Issue #133).

use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct SwarmNode {
    pub id: String,
    pub name: String,
    pub role: String,
    pub model: String,
    pub status: String,
    pub tokens_used: usize,
    pub subagents: Vec<SwarmNode>,
}

#[component]
pub fn SwarmView() -> Element {

    let swarm_tree = use_signal(|| SwarmNode {
        id: "lead-coordinator".to_string(),
        name: "Lead Supervisor Agent".to_string(),
        role: "Primary Coordinator & Task Dispatcher".to_string(),
        model: "claude-3-5-sonnet".to_string(),
        status: "Active".to_string(),
        tokens_used: 14_200,
        subagents: vec![
            SwarmNode {
                id: "sub-scout".to_string(),
                name: "Codebase Scout".to_string(),
                role: "AST Symbol Indexing & Tree-Sitter Search".to_string(),
                model: "claude-3-5-haiku".to_string(),
                status: "Idle".to_string(),
                tokens_used: 5_430,
                subagents: vec![],
            },
            SwarmNode {
                id: "sub-architect".to_string(),
                name: "Refactor Architect".to_string(),
                role: "Modular Interface Design & Seam Enforcement".to_string(),
                model: "claude-3-5-sonnet".to_string(),
                status: "Executing".to_string(),
                tokens_used: 18_900,
                subagents: vec![
                    SwarmNode {
                        id: "sub-qa".to_string(),
                        name: "QA & Verification Worker".to_string(),
                        role: "Cargo Test & Clippy Verification".to_string(),
                        model: "claude-3-5-haiku".to_string(),
                        status: "Idle".to_string(),
                        tokens_used: 3_120,
                        subagents: vec![],
                    }
                ],
            },
            SwarmNode {
                id: "sub-reviewer".to_string(),
                name: "Security & Governance Guard".to_string(),
                role: "Hook Policy & Workspace Isolation".to_string(),
                model: "gpt-4o".to_string(),
                status: "Active".to_string(),
                tokens_used: 6_800,
                subagents: vec![],
            }
        ],
    });

    let tree = swarm_tree();

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto flex flex-col justify-between select-text",
            // Header
            header { class: "px-8 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "flex items-center space-x-3",
                    span { class: "text-base font-bold text-white tracking-tight", "Swarm Topology & Supervisory Tree" }
                    span { class: "text-xs font-mono text-purple-400 bg-purple-950/60 border border-purple-800/80 px-2 py-0.5 rounded", "Multi-Agent Swarm Hierarchy" }
                }
                div { class: "flex items-center space-x-3 text-xs font-mono text-slate-400",
                    span { "Total Swarm Tokens: 48,450" }
                }
            }

            // Swarm Visual Topology Tree
            div { class: "p-8 max-w-6xl mx-auto space-y-8 flex-1",
                // Root Lead Node
                div { class: "flex justify-center",
                    render_swarm_card { node: tree.clone(), is_root: true }
                }

                // Subagent Branches Connector
                div { class: "w-0.5 h-8 bg-gradient-to-b from-purple-500 to-indigo-500 mx-auto" }

                // First Layer Subagents
                div { class: "grid grid-cols-1 md:grid-cols-3 gap-6",
                    for sub in tree.subagents.clone() {
                        div { class: "flex flex-col items-center space-y-4",
                            render_swarm_card { node: sub.clone(), is_root: false }
                            if !sub.subagents.is_empty() {
                                div { class: "w-0.5 h-6 bg-indigo-500/60" }
                                for leaf in sub.subagents.clone() {
                                    render_swarm_card { node: leaf, is_root: false }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn render_swarm_card(node: SwarmNode, is_root: bool) -> Element {
    let status_color = match node.status.as_str() {
        "Active" => "bg-emerald-400 animate-pulse",
        "Executing" => "bg-cyan-400 animate-pulse",
        _ => "bg-slate-500",
    };

    let border_color = if is_root {
        "border-purple-500/60 shadow-[0_0_20px_rgba(168,85,247,0.15)]"
    } else {
        "border-[#1e293b] hover:border-slate-600"
    };

    rsx! {
        div { class: "w-72 bg-[#090d16] border {border_color} rounded-xl p-5 shadow-xl flex flex-col justify-between transition-all duration-200",
            div {
                div { class: "flex items-center justify-between mb-2",
                    span { class: "text-slate-100 font-bold text-xs truncate", "{node.name}" }
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full {status_color}" }
                        span { class: "text-[10px] font-mono text-slate-400", "{node.status}" }
                    }
                }
                p { class: "text-slate-400 text-[11px] leading-relaxed mb-3", "{node.role}" }
            }
            div { class: "pt-3 border-t border-[#1e293b] flex items-center justify-between text-[10px] font-mono text-slate-500",
                span { "{node.model}" }
                span { "{node.tokens_used} tok" }
            }
        }
    }
}

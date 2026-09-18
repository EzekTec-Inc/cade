use dioxus::prelude::*;

use crate::types::{AppState, SelectedPage};

/// Left sidebar navigation with dynamic responsive width, collapsible toggle, and vertical scroll containment.
#[component]
pub fn Sidebar() -> Element {
    let state = use_context::<AppState>();
    let mut is_collapsed = use_signal(|| false);

    let collapsed = is_collapsed();
    let aside_width = if collapsed { "w-16" } else { "w-56 md:w-60" };

    rsx! {
        aside {
            class: "{aside_width} bg-[#090d16] border-r border-[#1e293b] flex flex-col justify-between h-full select-none text-sm shrink-0 font-sans transition-all duration-200 ease-in-out relative z-20",

            // Top Brand Header (Pinned, shrink-0)
            div { class: "p-3.5 flex items-center justify-between border-b border-[#1e293b]/70 shrink-0",
                div { class: "flex items-center space-x-2.5 min-w-0 overflow-hidden",
                    div { class: "w-7 h-7 rounded-lg bg-gradient-to-br from-orange-500 to-amber-600 flex items-center justify-center shadow-sm shadow-orange-500/20 shrink-0",
                        span { class: "text-white font-mono font-bold text-xs", "C" }
                    }
                    if !collapsed {
                        span { class: "font-semibold text-sm tracking-tight text-slate-100 truncate", "CADE" }
                        span { class: "bg-slate-800 text-[10px] text-slate-400 px-1.5 py-0.5 rounded font-mono font-medium border border-slate-700 shrink-0", "v0.2" }
                    }
                }
                button {
                    class: "p-1 rounded text-slate-500 hover:text-slate-200 hover:bg-slate-800/60 transition-colors text-xs shrink-0 cursor-pointer",
                    title: if collapsed { "Expand sidebar" } else { "Collapse sidebar" },
                    onclick: move |_| is_collapsed.set(!is_collapsed()),
                    if collapsed { "▶" } else { "◀" }
                }
            }

            // Project Selector (Only in expanded mode, shrink-0)
            if !collapsed {
                div { class: "p-3 shrink-0",
                    div { class: "bg-[#0f172a] border border-[#1e293b] rounded-lg p-2.5 flex items-center justify-between cursor-pointer hover:border-slate-600 transition-colors duration-150 shadow-sm",
                        div { class: "flex items-center space-x-2 min-w-0",
                            span { class: "text-slate-400 text-xs shrink-0", "⊞" }
                            span { class: "font-medium text-xs text-slate-200 truncate", "Default Workspace" }
                        }
                        span { class: "text-slate-500 text-[10px] shrink-0", "▼" }
                    }
                }
            }

            // Main navigation list with vertical scroll containment (flex-1 min-h-0 overflow-y-auto)
            nav { class: "flex-1 min-h-0 overflow-y-auto px-2 py-2 space-y-0.5",
                // Overview Group
                if !collapsed {
                    div { class: "text-[10px] font-bold text-slate-500 px-2.5 pt-2 pb-1 tracking-wider uppercase", "Overview" }
                }
                nav_item { active_page: state.active_page, page: SelectedPage::Dashboard, icon: "🎛", label: "Dashboard", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Live, icon: "🟢", label: "Live Activity", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Code, icon: "⌨", label: "Code", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Chat, icon: "💬", label: "Chat", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Arena, icon: "⚡", label: "Model Arena", collapsed }

                // Intelligence & Workflows Group
                if !collapsed {
                    div { class: "text-[10px] font-bold text-slate-500 px-2.5 pt-4 pb-1 tracking-wider uppercase", "Orchestration" }
                }
                nav_item { active_page: state.active_page, page: SelectedPage::Agents, icon: "🤖", label: "Agents", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Workflows, icon: "🔄", label: "Workflows DAG", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Swarm, icon: "🌐", label: "Swarm Topology", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Artifacts, icon: "📦", label: "Artifact Studio", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Logs, icon: "📋", label: "Event Logs", collapsed }

                // Resources Group
                if !collapsed {
                    div { class: "text-[10px] font-bold text-slate-500 px-2.5 pt-4 pb-1 tracking-wider uppercase", "Resources" }
                }
                nav_item { active_page: state.active_page, page: SelectedPage::MemoryBlocks, icon: "🧠", label: "Memory Blocks", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Tools, icon: "🛠", label: "Tools & Approvals", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Models, icon: "⚙", label: "Models", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Providers, icon: "📡", label: "Providers", collapsed }
            }

            // Bottom controls (Pinned, shrink-0)
            div { class: "p-2 border-t border-[#1e293b]/70 space-y-0.5 shrink-0 bg-[#090d16]",
                nav_item { active_page: state.active_page, page: SelectedPage::ApiKeys, icon: "🔑", label: "API Keys", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Usage, icon: "📊", label: "Telemetry & Cost", collapsed }
                nav_item { active_page: state.active_page, page: SelectedPage::Settings, icon: "⚙", label: "Settings", collapsed }
            }
        }
    }
}

/// A single navigation item in the sidebar.
#[component]
fn nav_item(
    active_page: Signal<SelectedPage>,
    page: SelectedPage,
    icon: String,
    label: String,
    collapsed: bool,
) -> Element {
    let is_active = active_page() == page;
    let cls = if is_active {
        "flex items-center justify-between px-2.5 py-2 rounded-lg bg-slate-800/90 text-sky-400 font-medium cursor-pointer border border-slate-700 shadow-sm transition-all"
    } else {
        "flex items-center justify-between px-2.5 py-2 rounded-lg text-slate-400 hover:text-slate-100 hover:bg-slate-800/40 cursor-pointer transition-colors duration-150"
    };

    rsx! {
        div {
            class: if collapsed { format!("{cls} justify-center") } else { cls.to_string() },
            title: "{label}",
            onclick: move |_| active_page.set(page),
            div { class: "flex items-center space-x-2.5 min-w-0",
                span { class: "text-sm shrink-0", "{icon}" }
                if !collapsed {
                    span { class: "text-xs truncate", "{label}" }
                }
            }
            if is_active && !collapsed {
                span { class: "w-1.5 h-1.5 rounded-full bg-sky-400 shrink-0" }
            }
        }
    }
}

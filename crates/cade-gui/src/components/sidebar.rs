use dioxus::prelude::*;

use crate::types::{AppState, SelectedPage};

/// Left sidebar navigation.
#[component]
pub fn Sidebar() -> Element {
    let state = use_context::<AppState>();

    rsx! {
        aside { class: "w-[240px] bg-[#090d16] border-r border-[#1e293b] flex flex-col justify-between h-full select-none text-sm shrink-0 font-sans",
            div { class: "flex flex-col",
                // Top Brand Header
                div { class: "p-4 flex items-center justify-between border-b border-[#1e293b]/70",
                    div { class: "flex items-center space-x-2.5",
                        div { class: "w-6 h-6 rounded bg-gradient-to-br from-orange-500 to-amber-600 flex items-center justify-center shadow-sm shadow-orange-500/20",
                            span { class: "text-white font-mono font-bold text-xs", "C" }
                        }
                        span { class: "font-semibold text-sm tracking-tight text-slate-100", "CADE" }
                        span { class: "bg-slate-800 text-[10px] text-slate-400 px-1.5 py-0.5 rounded font-mono font-medium border border-slate-700", "v0.2" }
                    }
                }

                // Project Selector Dropdown
                div { class: "p-3",
                    div { class: "bg-[#0f172a] border border-[#1e293b] rounded-lg p-2.5 flex items-center justify-between cursor-pointer hover:border-slate-600 transition-colors duration-150 shadow-sm",
                        div { class: "flex items-center space-x-2.5",
                            span { class: "text-slate-400 text-xs", "⊞" }
                            span { class: "font-medium text-xs text-slate-200", "Default Workspace" }
                        }
                        span { class: "text-slate-500 text-[10px]", "▼" }
                    }
                }

                // Main navigation list
                nav { class: "px-2.5 space-y-0.5",
                    // Overview Group
                    div { class: "text-[10px] font-bold text-slate-500 px-3 pt-3 pb-1 tracking-wider uppercase", "Overview" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Dashboard, icon: "🎛", label: "Dashboard" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Code, icon: "⌨", label: "Code" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Chat, icon: "💬", label: "Chat" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Arena, icon: "⚡", label: "Model Arena" }

                    // Intelligence & Workflows Group
                    div { class: "text-[10px] font-bold text-slate-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Orchestration" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Agents, icon: "🤖", label: "Agents" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Workflows, icon: "🔄", label: "Workflows DAG" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Swarm, icon: "🌐", label: "Swarm Topology" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Artifacts, icon: "📦", label: "Artifact Studio" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Logs, icon: "📋", label: "Event Logs" }

                    // Resources Group
                    div { class: "text-[10px] font-bold text-slate-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Resources" }
                    nav_item { active_page: state.active_page, page: SelectedPage::MemoryBlocks, icon: "🧠", label: "Memory Blocks" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Tools, icon: "🛠", label: "Tools & Approvals" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Models, icon: "⚙", label: "Models" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Providers, icon: "📡", label: "Providers" }
                }
            }

            // Bottom controls
            div { class: "p-2.5 border-t border-[#1e293b]/70 space-y-0.5",
                nav_item { active_page: state.active_page, page: SelectedPage::ApiKeys, icon: "🔑", label: "API Keys" }
                nav_item { active_page: state.active_page, page: SelectedPage::Usage, icon: "📊", label: "Telemetry & Cost" }
                nav_item { active_page: state.active_page, page: SelectedPage::Settings, icon: "⚙", label: "Settings" }
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
) -> Element {
    let is_active = active_page() == page;
    let cls = if is_active {
        "flex items-center justify-between px-3 py-2 rounded-lg bg-slate-800/90 text-sky-400 font-medium cursor-pointer border border-slate-700 shadow-sm transition-all"
    } else {
        "flex items-center justify-between px-3 py-2 rounded-lg text-slate-400 hover:text-slate-100 hover:bg-slate-800/40 cursor-pointer transition-colors duration-150"
    };

    rsx! {
        div {
            class: "{cls}",
            onclick: move |_| active_page.set(page),
            div { class: "flex items-center space-x-2.5",
                span { class: "text-sm", "{icon}" }
                span { class: "text-xs", "{label}" }
            }
            if is_active {
                span { class: "w-1.5 h-1.5 rounded-full bg-sky-400" }
            }
        }
    }
}

use dioxus::prelude::*;

use crate::types::{AppState, SelectedPage};

/// Left sidebar navigation.
#[component]
pub fn Sidebar() -> Element {
    let state = use_context::<AppState>();

    rsx! {
        aside { class: "w-[240px] bg-[#0f1115] border-r border-[#1f222b] flex flex-col justify-between h-full select-none text-sm shrink-0",
            div { class: "flex flex-col",
                // Top Brand Header
                div { class: "p-4 flex items-center justify-between border-b border-[#1f222b]",
                    div { class: "flex items-center space-x-2",
                        svg { class: "w-5 h-5 text-white fill-current", view_box: "0 0 24 24",
                            rect { x: "4", y: "4", width: "16", height: "16", rx: "3", fill: "#ff7c5c" }
                            rect { x: "8", y: "8", width: "8", height: "8", rx: "1.5", fill: "#0f1115" }
                        }
                        span { class: "font-semibold text-[15px] tracking-tight text-[#e5e7eb]", "CADE" }
                        span { class: "bg-[#1f222b] text-[10px] text-gray-400 px-1.5 py-0.5 rounded font-medium", "Beta" }
                    }
                }

                // Project Selector Dropdown
                div { class: "p-3",
                    div { class: "bg-[#16171d] border border-[#272833] rounded-md p-2 flex items-center justify-between cursor-pointer hover:bg-[#1f212a] transition duration-150",
                        div { class: "flex items-center space-x-2",
                            span { class: "text-gray-400 text-xs", "\u{229e}" }
                            span { class: "font-medium text-xs text-gray-200", "Default Project" }
                        }
                        span { class: "text-gray-500 text-[10px]", "\u{25bc}" }
                    }
                }

                // Main navigation list
                nav { class: "px-2 space-y-0.5",
                    // Dashboard Group
                    div { class: "text-[10px] font-bold text-gray-500 px-3 pt-3 pb-1 tracking-wider uppercase", "Dashboard" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Dashboard, icon: "\u{1f39b}", label: "Dashboard" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Code, icon: "\u{2328}", label: "Code" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Chat, icon: "\u{1f4ac}", label: "Chat" }

                    // Development Group
                    div { class: "text-[10px] font-bold text-gray-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Development" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Agents, icon: "\u{1f916}", label: "Agents" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Logs, icon: "\u{1f4cb}", label: "Logs" }

                    // Resources Group
                    div { class: "text-[10px] font-bold text-gray-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Resources" }
                    nav_item { active_page: state.active_page, page: SelectedPage::MemoryBlocks, icon: "\u{1f9e0}", label: "Memory blocks" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Tools, icon: "\u{1f6e0}", label: "Tools" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Models, icon: "\u{2699}", label: "Models" }
                    nav_item { active_page: state.active_page, page: SelectedPage::Providers, icon: "\u{1f4e1}", label: "Providers" }
                }
            }

            // Bottom controls
            div { class: "p-2 border-t border-[#1f222b] space-y-0.5",
                nav_item { active_page: state.active_page, page: SelectedPage::ApiKeys, icon: "\u{1f511}", label: "API Keys" }
                nav_item { active_page: state.active_page, page: SelectedPage::Usage, icon: "\u{1f4ca}", label: "Usage" }
                nav_item { active_page: state.active_page, page: SelectedPage::Settings, icon: "\u{2699}", label: "Settings" }
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
        "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer"
    } else {
        "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer"
    };

    rsx! {
        div {
            class: "{cls}",
            onclick: move |_| active_page.set(page),
            div { class: "flex items-center space-x-2.5",
                span { class: "text-sm", "{icon}" }
                span { "{label}" }
            }
        }
    }
}

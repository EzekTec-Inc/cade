use dioxus::prelude::*;
use serde_json::Value;

use crate::api_engine::{ApiClientEngine, ResourceMutation, ResourceState};
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn PluginSettings() -> Element {
    let engine = use_context::<Memo<ApiClientEngine>>();
    let app_state = use_context::<AppState>();
    let mut plugins = use_signal(|| ResourceState::<Vec<Value>>::Loading);
    let mut active_tab = use_signal(|| "installed");

    // Marketplace state
    let mut market_query = use_signal(String::new);
    let mut market_plugins = use_signal(|| ResourceState::<Vec<Value>>::Loading);

    // Initial load for installed plugins
    use_effect(move || {
        let eng = engine();
        spawn(async move {
            let res = eng.fetch_plugins().await;
            plugins.set(res);
        });
    });

    // Load marketplace plugins when tab switches to marketplace
    use_effect(move || {
        if active_tab() == "marketplace" {
            let eng = engine();
            let query = market_query();
            spawn(async move {
                market_plugins.set(ResourceState::Loading);
                let res = eng.search_marketplace(&query).await;
                market_plugins.set(res);
            });
        }
    });

    let on_refresh = move |_| {
        let eng = engine();
        spawn(async move {
            plugins.set(ResourceState::Loading);
            let res = eng.fetch_plugins().await;
            plugins.set(res);

            if active_tab() == "marketplace" {
                market_plugins.set(ResourceState::Loading);
                let mres = eng.search_marketplace(&market_query()).await;
                market_plugins.set(mres);
            }
        });
    };

    let on_market_search = move |_| {
        let eng = engine();
        let query = market_query();
        spawn(async move {
            market_plugins.set(ResourceState::Loading);
            let res = eng.search_marketplace(&query).await;
            market_plugins.set(res);
        });
    };

    let on_uninstall = move |id: String| {
        let eng = engine();
        spawn(async move {
            let res = eng
                .mutate(ResourceMutation::UninstallPlugin { plugin_id: id.clone() })
                .await;
            match res {
                Ok(_) => {
                    add_toast(&app_state, ToastLevel::Success, "Plugin Uninstalled", format!("{id} removed"));
                    let updated = eng.fetch_plugins().await;
                    plugins.set(updated);
                }
                Err(e) => {
                    add_toast(&app_state, ToastLevel::Error, "Uninstall Failed", e);
                }
            }
        });
    };

    let on_market_install = move |plugin_id: String| {
        let eng = engine();
        spawn(async move {
            add_toast(&app_state, ToastLevel::Info, "Installing Plugin", format!("Installing {plugin_id}..."));
            let res = eng
                .mutate(ResourceMutation::InstallPlugin {
                    url: plugin_id.clone(),
                    plugin_id: plugin_id.clone(),
                })
                .await;
            match res {
                Ok(_) => {
                    add_toast(&app_state, ToastLevel::Success, "Plugin Installed", format!("{plugin_id} installed successfully"));
                    let updated = eng.fetch_plugins().await;
                    plugins.set(updated);
                }
                Err(e) => {
                    add_toast(&app_state, ToastLevel::Error, "Installation Failed", e);
                }
            }
        });
    };

    rsx! {
        div { class: "flex flex-col h-full bg-[#181825] text-[#cdd6f4] p-6 overflow-y-auto",
            div { class: "flex justify-between items-center mb-6",
                div {
                    h1 { class: "text-2xl font-bold tracking-tight text-[#cdd6f4]", "Plugin Ecosystem" }
                    p { class: "text-sm text-[#a6adc8]", "Manage installed packages and explore the remote community marketplace." }
                }
                button {
                    class: "px-4 py-2 bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] rounded-lg transition duration-200 text-sm font-medium flex items-center gap-2",
                    onclick: on_refresh,
                    "Refresh"
                }
            }

            // Tab Navigation
            div { class: "flex gap-2 border-b border-[#313244] mb-6 pb-2",
                button {
                    class: if active_tab() == "installed" {
                        "px-4 py-2 bg-[#89b4fa]/20 text-[#89b4fa] font-semibold rounded-lg text-sm transition"
                    } else {
                        "px-4 py-2 text-[#a6adc8] hover:text-[#cdd6f4] text-sm transition"
                    },
                    onclick: move |_| active_tab.set("installed"),
                    "Installed Plugins"
                }
                button {
                    class: if active_tab() == "marketplace" {
                        "px-4 py-2 bg-[#89b4fa]/20 text-[#89b4fa] font-semibold rounded-lg text-sm transition"
                    } else {
                        "px-4 py-2 text-[#a6adc8] hover:text-[#cdd6f4] text-sm transition"
                    },
                    onclick: move |_| active_tab.set("marketplace"),
                    "Marketplace Catalog"
                }
            }

            if active_tab() == "installed" {
                // Installed Plugins View
                match &*plugins.read() {
                    ResourceState::Loading => rsx! {
                        div { class: "flex justify-center p-12 text-[#a6adc8]", "Loading installed plugin inventory..." }
                    },
                    ResourceState::Error(err) => rsx! {
                        div { class: "p-4 bg-[#f38ba8]/20 border border-[#f38ba8] text-[#f38ba8] rounded-lg",
                            "Failed to load plugins: {err}"
                        }
                    },
                    ResourceState::Ready(list) => {
                        if list.is_empty() {
                            rsx! {
                                div { class: "p-8 text-center text-[#a6adc8] bg-[#1e1e2e] rounded-xl border border-[#313244]",
                                    p { class: "mb-2", "No plugins currently installed." }
                                    p { class: "text-xs text-[#6c7086]", "Switch to the Marketplace Catalog tab to discover community plugins." }
                                }
                            }
                        } else {
                            rsx! {
                                div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                                    for item in list {
                                        {
                                            let id = item["id"].as_str().unwrap_or("unknown").to_string();
                                            let name = item["name"].as_str().unwrap_or(&id).to_string();
                                            let version = item["version"].as_str().unwrap_or("0.1.0").to_string();
                                            let scope = item["scope"].as_str().unwrap_or("global").to_string();
                                            let tools_cnt = item["tools_count"].as_u64().unwrap_or(0);
                                            let skills_cnt = item["skills_count"].as_u64().unwrap_or(0);
                                            let id_clone = id.clone();

                                            rsx! {
                                                div { key: "{id}", class: "bg-[#1e1e2e] border border-[#313244] rounded-xl p-5 flex flex-col justify-between hover:border-[#45475a] transition",
                                                    div {
                                                        div { class: "flex justify-between items-start mb-2",
                                                            h3 { class: "text-lg font-bold text-[#cdd6f4]", "{name}" }
                                                            span { class: "text-xs px-2 py-0.5 rounded bg-[#313244] text-[#a6adc8]", "v{version}" }
                                                        }
                                                        div { class: "flex items-center gap-2 mb-4 text-xs text-[#a6adc8]",
                                                            span { class: "uppercase tracking-wide px-1.5 py-0.5 rounded bg-[#45475a]/50 text-[#cdd6f4]", "{scope}" }
                                                            span { "•" }
                                                            span { "{tools_cnt} tools" }
                                                            span { "•" }
                                                            span { "{skills_cnt} skills" }
                                                        }
                                                    }
                                                    div { class: "flex justify-end pt-3 border-t border-[#313244]/60",
                                                        button {
                                                            class: "px-3 py-1.5 bg-[#f38ba8]/20 hover:bg-[#f38ba8]/30 text-[#f38ba8] text-xs font-semibold rounded transition",
                                                            onclick: move |_| on_uninstall(id_clone.clone()),
                                                            "Uninstall"
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
            } else {
                div { class: "flex flex-col gap-4",
                    // Search bar
                    div { class: "flex gap-2",
                        input {
                            class: "flex-1 px-4 py-2.5 bg-[#1e1e2e] border border-[#313244] rounded-lg text-sm text-[#cdd6f4] placeholder-[#6c7086] focus:outline-none focus:border-[#89b4fa]",
                            placeholder: "Search marketplace plugins (e.g. rust, lsp, python, git)...",
                            value: "{market_query}",
                            oninput: move |evt| market_query.set(evt.value().clone()),
                        }
                        button {
                            class: "px-5 py-2.5 bg-[#89b4fa] hover:bg-[#b4befe] text-[#11111b] font-semibold text-sm rounded-lg transition",
                            onclick: on_market_search,
                            "Search Catalog"
                        }
                    }

                    // Marketplace Results
                    match &*market_plugins.read() {
                        ResourceState::Loading => rsx! {
                            div { class: "flex justify-center p-12 text-[#a6adc8]", "Querying marketplace registry..." }
                        },
                        ResourceState::Error(err) => rsx! {
                            div { class: "p-4 bg-[#f38ba8]/20 border border-[#f38ba8] text-[#f38ba8] rounded-lg",
                                "Marketplace catalog error: {err}"
                            }
                        },
                        ResourceState::Ready(list) => {
                            if list.is_empty() {
                                rsx! {
                                    div { class: "p-8 text-center text-[#a6adc8] bg-[#1e1e2e] rounded-xl border border-[#313244]",
                                        "No marketplace plugins found matching '{market_query}'."
                                    }
                                }
                            } else {
                                rsx! {
                                    div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                                        for item in list {
                                            {
                                                let id = item["id"].as_str().unwrap_or("unknown").to_string();
                                                let version = item["version"].as_str().unwrap_or("0.1.0").to_string();
                                                let desc = item["description"].as_str().unwrap_or("").to_string();
                                                let author = item["author"].as_str().unwrap_or("community").to_string();
                                                let tags = item["tags"].as_array().cloned().unwrap_or_default();
                                                let id_clone = id.clone();

                                                rsx! {
                                                    div { key: "{id}", class: "bg-[#1e1e2e] border border-[#313244] rounded-xl p-5 flex flex-col justify-between hover:border-[#89b4fa]/50 transition",
                                                        div {
                                                            div { class: "flex justify-between items-start mb-2",
                                                                div {
                                                                    h3 { class: "text-lg font-bold text-[#cdd6f4]", "{id}" }
                                                                    span { class: "text-xs text-[#a6adc8]", "by {author}" }
                                                                }
                                                                span { class: "text-xs px-2 py-0.5 rounded bg-[#313244] text-[#a6adc8]", "v{version}" }
                                                            }
                                                            p { class: "text-sm text-[#bac2de] mb-3 line-clamp-2", "{desc}" }
                                                            if !tags.is_empty() {
                                                                div { class: "flex flex-wrap gap-1.5 mb-3",
                                                                    for tag in tags {
                                                                        span { class: "text-xs px-2 py-0.5 bg-[#313244]/80 text-[#89dceb] rounded",
                                                                            "{tag.as_str().unwrap_or(\"\")}"
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        div { class: "flex justify-end pt-3 border-t border-[#313244]/60",
                                                            button {
                                                                class: "px-4 py-1.5 bg-[#a6e3a1] hover:bg-[#94e2d5] text-[#11111b] text-xs font-bold rounded transition",
                                                                onclick: move |_| on_market_install(id_clone.clone()),
                                                                "Install"
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
            }
        }
    }
}

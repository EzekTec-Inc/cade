use dioxus::prelude::*;

use crate::api_engine::{ApiClientEngine, ResourceMutation, ResourceState};
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn PluginSettings() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let engine = ApiClientEngine::new(client);
    let plugins = use_signal(|| ResourceState::Loading);
    let mut install_url = use_signal(String::new);
    let mut install_id = use_signal(String::new);
    let busy = use_signal(|| false);

    let engine_for_load = engine.clone();
    let load_plugins = move || {
        let engine = engine_for_load.clone();
        let mut plugins = plugins;
        spawn(async move { plugins.set(engine.fetch_plugins().await) });
    };
    let effect_load_plugins = load_plugins.clone();
    use_effect(move || effect_load_plugins());

    let content = match plugins() {
        ResourceState::Loading => rsx! {
            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 animate-pulse",
                div { class: "h-4 bg-[#272833] rounded w-1/4 mb-3" }
                div { class: "h-3 bg-[#272833] rounded w-2/3" }
            }
        },
        ResourceState::Error(error) => rsx! {
            div { class: "bg-[#090d16] border border-red-900/50 rounded-xl p-5 text-red-300 text-sm", "Plugin inventory unavailable: {error}" }
        },
        ResourceState::Ready(plugin_list) if plugin_list.is_empty() => rsx! {
            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 text-slate-500 text-sm", "No active plugins." }
        },
        ResourceState::Ready(plugin_list) => rsx! {
            div { class: "space-y-2",
                for plugin in plugin_list {
                    plugin_card { plugin, engine: engine.clone(), plugins, busy, state }
                }
            }
        },
    };

    rsx! {
        div { class: "space-y-3",
            div { class: "flex items-center justify-between",
                h2 { class: "text-sm font-semibold text-slate-100", "Plugins" }
                button {
                    class: "text-xs text-slate-400 hover:text-slate-100 disabled:opacity-50",
                    disabled: busy(),
                    onclick: move |_| load_plugins(),
                    "Refresh"
                }
            }
            {content}
            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 space-y-3",
                p { class: "text-slate-400 text-xs", "Install a validated Plugin package by URL and stable identifier." }
                div { class: "flex gap-2",
                    input {
                        class: "flex-1 bg-[#1f212a] text-slate-100 text-xs border border-[#1e293b] rounded-md px-2 py-1.5 outline-none focus:border-[#ff7c5c]",
                        placeholder: "Package URL",
                        value: "{install_url}",
                        oninput: move |event| install_url.set(event.value()),
                    }
                    input {
                        class: "w-40 bg-[#1f212a] text-slate-100 text-xs border border-[#1e293b] rounded-md px-2 py-1.5 outline-none focus:border-[#ff7c5c]",
                        placeholder: "Plugin id",
                        value: "{install_id}",
                        oninput: move |event| install_id.set(event.value()),
                    }
                    button {
                        class: "bg-[#ff7c5c] hover:bg-[#ff906f] text-[#141414] text-xs font-semibold rounded-md px-3 py-1.5 disabled:opacity-50",
                        disabled: busy() || install_url().trim().is_empty() || install_id().trim().is_empty(),
                        onclick: move |_| {
                            let url = install_url().trim().to_string();
                            let plugin_id = install_id().trim().to_string();
                            let engine = engine.clone();
                            let mut plugins = plugins;
                            let mut busy = busy;
                            let st = state;
                            busy.set(true);
                            spawn(async move {
                                match engine.mutate(ResourceMutation::InstallPlugin { url, plugin_id }).await {
                                    Ok(id) => {
                                        add_toast(&st, ToastLevel::Success, "Plugin installed", id);
                                        plugins.set(engine.fetch_plugins().await);
                                    }
                                    Err(error) => add_toast(&st, ToastLevel::Error, "Plugin installation failed", error),
                                }
                                busy.set(false);
                            });
                        },
                        "Install"
                    }
                }
            }
        }
    }
}

#[component]
fn plugin_card(
    plugin: serde_json::Value,
    engine: ApiClientEngine,
    mut plugins: Signal<ResourceState<Vec<serde_json::Value>>>,
    mut busy: Signal<bool>,
    state: AppState,
) -> Element {
    let id = plugin["id"].as_str().unwrap_or("unknown").to_string();
    let name = plugin["name"].as_str().unwrap_or(&id).to_string();
    let version = plugin["version"].as_str().unwrap_or("unknown").to_string();
    let scope = plugin["scope"].as_str().unwrap_or("unknown").to_string();
    let tools = plugin["tools_count"].as_u64().unwrap_or(0);

    rsx! {
        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 flex items-center justify-between gap-4",
            div { class: "min-w-0",
                p { class: "text-slate-100 text-sm font-medium truncate", "{name}" }
                p { class: "text-slate-500 text-xs font-mono", "{id} · v{version} · {scope} · {tools} tools" }
            }
            if scope == "project" {
                button {
                    class: "text-red-300 hover:text-red-200 text-xs border border-red-900/60 hover:border-red-700 rounded-md px-2 py-1 disabled:opacity-50",
                    disabled: busy(),
                    onclick: move |_| {
                        let plugin_id = id.clone();
                        let engine = engine.clone();
                        let mut busy = busy;
                        let st = state;
                        busy.set(true);
                        spawn(async move {
                            match engine.mutate(ResourceMutation::UninstallPlugin { plugin_id }).await {
                                Ok(removed) => {
                                    add_toast(&st, ToastLevel::Success, "Plugin removed", removed);
                                    plugins.set(engine.fetch_plugins().await);
                                }
                                Err(error) => add_toast(&st, ToastLevel::Error, "Plugin removal failed", error),
                            }
                            busy.set(false);
                        });
                    },
                    "Remove"
                }
            }
        }
    }
}

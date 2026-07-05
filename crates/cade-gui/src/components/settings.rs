use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

fn save_agent_name(
    edit_name: Signal<String>,
    agent_id: String,
    agent_name: String,
    mut saving: Signal<bool>,
    api_key: Signal<String>,
    state: AppState,
    mut editing: Signal<bool>,
) {
    let new_name = edit_name().trim().to_string();
    if new_name.is_empty() || new_name == agent_name {
        editing.set(false);
        return;
    }
    saving.set(true);
    let k = api_key();
    let st = state;
    spawn(async move {
        let body = serde_json::json!({ "name": new_name }).to_string();
        let path = format!("/v1/agents/{agent_id}");
        match api::api_request("PATCH", &path, Some(&body), &k).await {
            Ok(_) => {
                add_toast(&st, ToastLevel::Success, "Agent updated", &new_name);
                editing.set(false);
            }
            Err(e) => {
                add_toast(&st, ToastLevel::Error, "Failed to update agent", e);
            }
        }
        saving.set(false);
    });
}

#[component]
pub fn SettingsView() -> Element {
    let state = use_context::<AppState>();

    let config_data = use_signal(|| Option::<serde_json::Value>::None);
    let agents = use_signal(Vec::<cade_api_types::AgentInfo>::new);
    let fetching = use_signal(|| true);

    let key = state.api_key;
    use_effect(move || {
        let k = key;
        let st = state;
        let mut cfg = config_data;
        let mut ags = agents;
        let mut busy = fetching;
        spawn(async move {
            match api::get_config(&k()).await {
                Ok(c) => cfg.set(Some(c)),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch config", e),
            }
            match api::list_agents(&k()).await {
                Ok(list) => ags.set(list),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch agents", e),
            }
            busy.set(false);
        });
    });

    let config_section = config_data().as_ref().map(|cfg| {
        let provider = cfg.get("provider").and_then(|v| v.as_str()).unwrap_or("—").to_string();
        let default_model = cfg.get("default_model").and_then(|v| v.as_str()).unwrap_or("—").to_string();
        let version = cfg.get("version").and_then(|v| v.as_str()).unwrap_or("—").to_string();
        let available: Vec<String> = cfg.get("available_providers")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        rsx! {
            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 space-y-4",
                config_row { label: "Version", value: version }
                config_row { label: "Provider", value: provider }
                config_row { label: "Default Model", value: default_model }
                if !available.is_empty() {
                    div { class: "flex items-center justify-between",
                        span { class: "text-gray-400 text-xs", "Available Providers" }
                        div { class: "flex gap-1.5",
                            {available.iter().map(|p| {
                                let name = p.clone();
                                rsx! {
                                    span { class: "bg-[#1f212a] text-gray-300 text-[10px] border border-[#272833] rounded px-1.5 py-0.5 font-mono", "{name}" }
                                }
                            })}
                        }
                    }
                }
            }
        }
    });

    let key_val = (state.api_key)();
    let masked = if key_val.len() > 8 {
        format!("{}\u{2026}{}", &key_val[..4], &key_val[key_val.len() - 4..])
    } else {
        "(not set)".to_string()
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Settings" }
            }

            div { class: "p-10 space-y-8 max-w-3xl",
                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Server Configuration" }
                    if fetching() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-1/2 mb-2" }
                            div { class: "h-3 bg-[#272833] rounded w-2/3" }
                        }
                    } else if let Some(ref s) = config_section {
                        {s.clone()}
                    }
                }

                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Agents" }
                    if fetching() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 animate-pulse",
                            div { class: "h-4 bg-[#272833] rounded w-1/4 mb-3" }
                            div { class: "h-3 bg-[#272833] rounded w-1/2" }
                        }
                    } else if agents().is_empty() {
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 text-center",
                            p { class: "text-gray-500 text-sm", "No agents configured." }
                        }
                    } else {
                        div { class: "space-y-2",
                            for a in agents().iter() {
                                agent_settings_card { agent: a.clone(), api_key: state.api_key }
                            }
                        }
                    }
                }

                div { class: "space-y-3",
                    h2 { class: "text-sm font-semibold text-white", "Authentication" }
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 space-y-3",
                        config_row { label: "API Key", value: masked }
                        p { class: "text-gray-600 text-xs", "The API key is stored in-memory for the session." }
                    }
                }
            }
        }
    }
}

#[component]
fn config_row(label: String, value: String) -> Element {
    rsx! {
        div { class: "flex items-center justify-between",
            span { class: "text-gray-400 text-xs", "{label}" }
            span { class: "text-white text-xs font-mono", "{value}" }
        }
    }
}

#[component]
fn agent_settings_card(agent: cade_api_types::AgentInfo, api_key: Signal<String>) -> Element {
    let mut editing = use_signal(|| false);
    let mut edit_name = use_signal(|| agent.name.clone());
    let saving = use_signal(|| false);

    let state = use_context::<AppState>();
    let agent_id = agent.id.clone();
    let agent_name = agent.name.clone();

    rsx! {
        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5",
            div { class: "flex items-center justify-between",
                div { class: "flex flex-col space-y-1",
                    if editing() {
                        div { class: "flex items-center space-x-2",
                            input {
                                class: "bg-[#1f212a] text-white text-sm border border-[#272833] rounded-md px-2 py-1 outline-none focus:border-[#ff7c5c]",
                                value: "{edit_name}",
                                oninput: move |e| edit_name.set(e.value().clone()),
                                onkeydown: {
                                    let aid = agent_id.clone();
                                    let aname = agent_name.clone();
                                    move |e| {
                                        if e.key() == Key::Enter {
                                            save_agent_name(
                                                edit_name, aid.clone(), aname.clone(),
                                                saving, api_key, state, editing,
                                            );
                                        } else if e.key() == Key::Escape {
                                            editing.set(false);
                                        }
                                    }
                                }
                            }
                            button {
                                class: "text-[10px] bg-[#ff7c5c] text-white rounded px-2 py-1 hover:bg-[#e26a4f] transition",
                                disabled: saving(),
                                onclick: {
                                    let aid = agent_id.clone();
                                    let aname = agent_name.clone();
                                    move |_| {
                                        save_agent_name(
                                            edit_name, aid.clone(), aname.clone(),
                                            saving, api_key, state, editing,
                                        );
                                    }
                                },
                                if saving() { "..." } else { "Save" }
                            }
                            button {
                                class: "text-[10px] text-gray-400 hover:text-white transition",
                                onclick: move |_| editing.set(false),
                                "Cancel"
                            }
                        }
                    } else {
                        span { class: "text-white font-semibold text-sm", "{agent.name}" }
                    }
                    div { class: "flex items-center space-x-2 text-xs text-gray-500",
                        if let Some(ref m) = agent.model {
                            span { class: "font-mono", "{m}" }
                        }
                        if let Some(ref p) = agent.provider {
                            span { "\u{2022}" }
                            span { "{p}" }
                        }
                    }
                }
                if !editing() {
                    button {
                        class: "text-gray-500 hover:text-white text-xs transition",
                        onclick: move |_| editing.set(true),
                        "Edit"
                    }
                }
            }
        }
    }
}

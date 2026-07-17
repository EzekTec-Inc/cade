use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

/// Provider management page — list, add, delete providers.
#[component]
pub fn ProvidersView() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let providers = use_signal(Vec::<serde_json::Value>::new);
    let presets = use_signal(Vec::<serde_json::Value>::new);
    let mut show_add = use_signal(|| false);
    let mut form_name = use_signal(String::new);
    let mut form_kind = use_signal(String::new);
    let mut form_api_key_val = use_signal(String::new);
    let mut form_base_url = use_signal(String::new);
    let mut form_model = use_signal(String::new);
    let mut error_msg = use_signal(String::new);
    let fetching = use_signal(|| true);

    // Fetch providers + presets on mount
    use_effect(move || {
        let st = state;
        let api_client = client();
        let mut provs = providers;
        let mut pres = presets;
        let mut busy = fetching;
        spawn(async move {
            match api_client.list_providers().await {
                Ok(data) => {
                    if let Some(arr) = data.get("providers").and_then(|v| v.as_array()) {
                        provs.set(arr.clone());
                    }
                }
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch providers", e),
            }
            match api_client.list_presets().await {
                Ok(data) => {
                    if let Some(arr) = data.get("presets").and_then(|v| v.as_array()) {
                        pres.set(arr.clone());
                    }
                }
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to fetch presets", e),
            }
            busy.set(false);
        });
    });

    let mut add_provider = move || {
        let name = form_name().trim().to_string();
        let kind = form_kind().trim().to_string();
        if name.is_empty() || kind.is_empty() {
            error_msg.set("Name and Kind are required".to_string());
            return;
        }
        let api_key_val = {
            let v = form_api_key_val();
            if v.trim().is_empty() {
                None
            } else {
                Some(v.trim().to_string())
            }
        };
        let base_url = {
            let v = form_base_url();
            if v.trim().is_empty() {
                None
            } else {
                Some(v.trim().to_string())
            }
        };
        let api_client = client();
        let mut provs = providers;
        let st = state;

        spawn(async move {
            match api_client
                .add_provider(&name, &kind, api_key_val.as_deref(), base_url.as_deref())
                .await
            {
                Ok(_) => {
                    match api_client.list_providers().await {
                        Ok(data) => {
                            if let Some(arr) = data.get("providers").and_then(|v| v.as_array()) {
                                provs.set(arr.clone());
                            }
                        }
                        Err(e) => {
                            add_toast(&st, ToastLevel::Error, "Failed to refresh providers", e)
                        }
                    }
                    form_name.set(String::new());
                    form_kind.set(String::new());
                    form_api_key_val.set(String::new());
                    form_base_url.set(String::new());
                    form_model.set(String::new());
                    show_add.set(false);
                    error_msg.set(String::new());
                    add_toast(&st, ToastLevel::Success, "Provider added", &name);
                }
                Err(e) => {
                    error_msg.set(e.clone());
                    add_toast(&st, ToastLevel::Error, "Failed to add provider", e);
                }
            }
        });
    };

    let remove_provider = move |name: String| {
        let api_client = client();
        let mut provs = providers;
        let st = state;
        spawn(async move {
            match api_client.remove_provider(&name).await {
                Ok(_) => {
                    add_toast(&st, ToastLevel::Success, "Provider removed", &name);
                }
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Failed to remove provider", e);
                }
            }
            match api_client.list_providers().await {
                Ok(data) => {
                    if let Some(arr) = data.get("providers").and_then(|v| v.as_array()) {
                        provs.set(arr.clone());
                    }
                }
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to refresh providers", e),
            }
        });
    };

    // Pre-compute preset button data outside RSX (let not allowed in rsx!)
    let preset_buttons: Vec<(String, serde_json::Value)> = presets()
        .iter()
        .map(|p| {
            let label = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            (label, p.clone())
        })
        .collect();

    let mut apply_preset = move |preset: serde_json::Value| {
        if let Some(kind) = preset.get("kind").and_then(|v| v.as_str()) {
            form_kind.set(kind.to_string());
        }
        if let Some(url) = preset.get("base_url").and_then(|v| v.as_str()) {
            form_base_url.set(url.to_string());
        }
        if let Some(name) = preset.get("name").and_then(|v| v.as_str())
            && form_name().is_empty()
        {
            form_name.set(name.to_string());
        }
        if let Some(model) = preset.get("default_model").and_then(|v| v.as_str()) {
            form_model.set(model.to_string());
        }
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            // Header
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "Providers" }
                button {
                    class: "bg-[#ff7c5c] text-white text-xs font-semibold px-4 py-2 rounded-lg hover:bg-[#e26a4f] transition",
                    onclick: move |_| show_add.set(!show_add()),
                    if show_add() { "Cancel" } else { "+ Add provider" }
                }
            }

            div { class: "p-10 space-y-6",
                // Add provider form
                if show_add() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-6 space-y-4",
                        h2 { class: "text-sm font-semibold text-white", "New Provider" }

                        if !error_msg().is_empty() {
                            div { class: "bg-red-500/10 border border-red-500/40 rounded-lg p-3 text-xs text-red-400",
                                "{error_msg}"
                            }
                        }

                        // Preset quick-select
                            if !preset_buttons.is_empty() {
                                div { class: "flex flex-wrap gap-2",
                                    span { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase self-center", "Presets:" }
                                    for (label, p) in preset_buttons {
                                        button {
                                            class: "text-xs bg-[#1f212a] text-gray-300 border border-[#272833] rounded-md px-2.5 py-1 hover:bg-[#272a35] hover:text-white transition",
                                            onclick: move |_| apply_preset(p.clone()),
                                            "{label}"
                                        }
                                    }
                                }
                            }

                        div { class: "grid grid-cols-2 gap-4",
                            div { class: "flex flex-col space-y-1.5",
                                label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "Name *" }
                                input {
                                    class: "bg-[#1f212a] text-white text-sm rounded-md px-3 py-2 outline-none border border-[#272833]",
                                    placeholder: "my-provider",
                                    value: "{form_name}",
                                    oninput: move |e| form_name.set(e.value().clone()),
                                }
                            }
                            div { class: "flex flex-col space-y-1.5",
                                label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "Kind *" }
                                input {
                                    class: "bg-[#1f212a] text-white text-sm rounded-md px-3 py-2 outline-none border border-[#272833]",
                                    placeholder: "openrouter / openai / groq / custom",
                                    value: "{form_kind}",
                                    oninput: move |e| form_kind.set(e.value().clone()),
                                }
                            }
                            div { class: "flex flex-col space-y-1.5",
                                label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "API Key" }
                                input {
                                    class: "bg-[#1f212a] text-white text-sm rounded-md px-3 py-2 outline-none border border-[#272833]",
                                    placeholder: "sk-...",
                                    value: "{form_api_key_val}",
                                    oninput: move |e| form_api_key_val.set(e.value().clone()),
                                }
                            }
                            div { class: "flex flex-col space-y-1.5",
                                label { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase", "Base URL" }
                                input {
                                    class: "bg-[#1f212a] text-white text-sm rounded-md px-3 py-2 outline-none border border-[#272833]",
                                    placeholder: "https://api.openai.com/v1",
                                    value: "{form_base_url}",
                                    oninput: move |e| form_base_url.set(e.value().clone()),
                                }
                            }
                        }

                        button {
                            class: "self-start bg-[#ff7c5c] text-white text-xs font-semibold px-6 py-2 rounded-lg hover:bg-[#e26a4f] transition",
                            onclick: move |_| add_provider(),
                            "Save Provider"
                        }
                    }
                }

                // Provider list header
                h2 { class: "text-sm font-semibold text-white", "Configured Providers" }
                if fetching() {
                    div { class: "space-y-3",
                        for _ in 0..2 {
                            div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 animate-pulse",
                                div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                                div { class: "h-3 bg-[#272833] rounded w-1/2 mb-2" }
                                div { class: "h-3 bg-[#272833] rounded w-2/3" }
                            }
                        }
                    }
                } else if providers().is_empty() {
                    div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center",
                        p { class: "text-gray-500 text-sm", "No providers configured yet." }
                        p { class: "text-gray-600 text-xs mt-1", "Add one using the form above." }
                    }
                } else {
                    div { class: "space-y-3",
                        for p in providers().iter() {
                            provider_card {
                                data: p.clone(),
                                remove: move |name: String| remove_provider(name),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn provider_card(data: serde_json::Value, remove: EventHandler<String>) -> Element {
    let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    let kind = data.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let base_url = data.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
    let model = data.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let enabled = data
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let n = name.to_string();

    rsx! {
        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-5 flex items-center justify-between",
            div { class: "flex flex-col space-y-1",
                div { class: "flex items-center space-x-3",
                    span { class: "text-white font-semibold text-sm", "{name}" }
                    if enabled {
                        span { class: "text-[10px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/30 px-1.5 py-0.5 rounded", "active" }
                    } else {
                        span { class: "text-[10px] bg-gray-500/10 text-gray-400 border border-gray-500/30 px-1.5 py-0.5 rounded", "disabled" }
                    }
                }
                div { class: "flex items-center space-x-3 text-xs text-gray-400",
                    span { "{kind}" }
                    if !base_url.is_empty() {
                        span { "\u{2022}" }
                        span { class: "font-mono", "{base_url}" }
                    }
                }
                if !model.is_empty() {
                    div { class: "text-xs text-gray-500",
                        span { "Model: {model}" }
                    }
                }
            }
            button {
                class: "text-gray-500 hover:text-red-400 text-sm px-2 py-1 rounded hover:bg-red-500/10 transition",
                onclick: move |_| remove.call(n.clone()),
                "\u{2716} Delete"
            }
        }
    }
}

use dioxus::prelude::*;

use crate::api;
use crate::types::AppState;

/// Full-screen login overlay shown when no API key is configured.
///
/// The user pastes their CADE API key, which is stored in-memory
/// for the session (never persisted to localStorage/IndexedDB).
#[component]
pub fn LoginScreen() -> Element {
    let mut state = use_context::<AppState>();
    let mut input_key = use_signal(String::new);
    let mut error = use_signal(String::new);

    let mut do_login = move |key: String| {
        let key = key.trim().to_string();
        if key.is_empty() {
            error.set("Please enter an API key".to_string());
            return;
        }
        // Validate by listing agents
        let k = key.clone();
        spawn(async move {
            match api::list_agents(&k).await {
                Ok(_) => {
                    state.api_key.set(k);
                    error.set(String::new());
                }
                Err(e) => {
                    error.set(format!("Connection failed: {e}"));
                }
            }
        });
    };

    rsx! {
        div { class: "w-full h-full flex items-center justify-center bg-[#0f1115]",
            div { class: "flex flex-col items-center space-y-6 max-w-md w-full px-8",
                // Brand
                div { class: "flex items-center space-x-3 mb-4",
                    svg { class: "w-8 h-8 text-white fill-current", view_box: "0 0 24 24",
                        rect { x: "4", y: "4", width: "16", height: "16", rx: "3", fill: "#ff7c5c" }
                        rect { x: "8", y: "8", width: "8", height: "8", rx: "1.5", fill: "#0f1115" }
                    }
                    span { class: "text-2xl font-bold text-white tracking-tight", "CADE" }
                    span { class: "bg-[#1f222b] text-xs text-gray-400 px-2 py-0.5 rounded font-medium", "Dashboard" }
                }

                h1 { class: "text-xl font-semibold text-white text-center",
                    "Welcome to CADE Dashboard"
                }
                p { class: "text-gray-400 text-sm text-center leading-6",
                    "Enter your CADE API key to connect to the server. "
                    "Your key stays in-memory and is never persisted."
                }

                // API key input
                input {
                    class: "w-full bg-[#16171d] border border-[#272833] rounded-lg px-4 py-3 text-gray-200 placeholder-gray-500 outline-none focus:border-[#ff7c5c] transition text-sm",
                    "type": "password",
                    placeholder: "sk-cade-...",
                    value: "{input_key}",
                    oninput: move |e| input_key.set(e.value().clone()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            do_login(input_key());
                        }
                    }
                }

                // Error message
                if !error().is_empty() {
                    p { class: "text-red-400 text-xs text-center", "{error}" }
                }

                // Submit button
                button {
                    class: "w-full bg-[#ff7c5c] hover:bg-[#e26a4f] text-white font-semibold py-3 rounded-lg transition duration-150 text-sm",
                    onclick: move |_| do_login(input_key()),
                    "Connect"
                }
            }
        }
    }
}

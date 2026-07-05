use dioxus::prelude::*;

use crate::types::AppState;

#[component]
pub fn ApiKeysView() -> Element {
    let state = use_context::<AppState>();
    let key = (state.api_key)();
    let masked = if key.len() > 8 {
        format!("{}\u{2026}{}", &key[..4], &key[key.len() - 4..])
    } else {
        "(not set)".to_string()
    };

    rsx! {
        div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                h1 { class: "text-lg font-semibold text-white", "API Keys" }
            }
            div { class: "p-10",
                div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 max-w-lg",
                    h2 { class: "text-white font-semibold text-sm mb-4", "Server Authentication" }
                    p { class: "text-gray-400 text-xs mb-4",
                        "CADE uses a single server-level bearer token for API authentication. \
                         Set the CADE_API_KEY environment variable or configure it in the server settings file. \
                         There is no runtime API key management \u{2014} the key is fixed at server startup."
                    }
                    div { class: "bg-[#1f212a] border border-[#272833] rounded-lg p-3",
                        div { class: "text-[10px] font-bold text-gray-500 tracking-wider uppercase mb-1", "Current Key" }
                        span { class: "text-gray-300 text-sm font-mono", "{masked}" }
                    }
                    p { class: "text-gray-600 text-xs mt-4",
                        "To change the key, restart the server with a new CADE_API_KEY value."
                    }
                }
            }
        }
    }
}

use dioxus::prelude::*;

use crate::api;
use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn ApiKeysView() -> Element {
    let state = use_context::<AppState>();
    let key = (state.api_key)();
    let mut show_plain = use_signal(|| false);
    let testing = use_signal(|| false);

    let masked = if key.len() > 8 {
        format!("{}\u{2026}{}", &key[..4], &key[key.len() - 4..])
    } else if key.is_empty() {
        "(not set)".to_string()
    } else {
        "••••••••".to_string()
    };

    let display_key = if show_plain() { key.clone() } else { masked };

    let test_connection = move || {
        let k = key.clone();
        let st = state;
        let mut busy = testing;
        busy.set(true);

        spawn(async move {
            match api::list_agents(&k).await {
                Ok(agents) => {
                    add_toast(
                        &st,
                        ToastLevel::Success,
                        "Connection Verified",
                        format!("Successfully authenticated. {} agents available.", agents.len()),
                    );
                }
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Authentication Failed", e);
                }
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text",
            header { class: "px-10 py-5 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "space-y-1",
                    h1 { class: "text-lg font-bold text-slate-100 tracking-tight", "API Keys & Authentication" }
                    p { class: "text-xs text-slate-400", "Manage server authentication token and connection verification." }
                }
            }
            div { class: "p-10 max-w-2xl space-y-6",
                div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-6 space-y-5 shadow-xl",
                    div { class: "flex items-center justify-between border-b border-[#1e293b]/50 pb-4",
                        div { class: "space-y-0.5",
                            h2 { class: "text-slate-100 font-bold text-sm", "Server Bearer Token" }
                            p { class: "text-xs text-slate-400", "Used for authenticating all GUI and CLI requests to the local CADE daemon." }
                        }
                        span { class: "text-[10px] font-bold px-2 py-0.5 rounded border border-emerald-500/20 bg-emerald-500/10 text-emerald-400 font-mono",
                            "ACTIVE"
                        }
                    }

                    div { class: "bg-[#040711] border border-[#1e293b] rounded-lg p-4 space-y-2",
                        div { class: "flex items-center justify-between text-[11px] font-mono text-slate-500 uppercase tracking-wider",
                            span { "Authorization Bearer Token" }
                            button {
                                class: "text-xs text-slate-400 hover:text-slate-200 transition cursor-pointer select-none",
                                onclick: move |_| show_plain.set(!show_plain()),
                                if show_plain() { "Hide" } else { "Reveal" }
                            }
                        }
                        div { class: "text-slate-200 font-mono text-sm tracking-wide break-all select-all py-1",
                            "{display_key}"
                        }
                    }

                    div { class: "flex flex-wrap items-center gap-3 pt-2",
                        button {
                            class: "text-xs bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg px-4 py-2 font-medium transition shadow-md flex items-center space-x-2 disabled:opacity-50 cursor-pointer",
                            disabled: testing(),
                            onclick: move |_| test_connection(),
                            if testing() {
                                span { "Verifying..." }
                            } else {
                                span { "⚡ Test Connection" }
                            }
                        }
                    }

                    div { class: "pt-4 border-t border-[#1e293b]/40 text-xs text-slate-400 leading-relaxed space-y-1",
                        p { "To rotate or set a new bearer token, configure the " code { class: "text-cyan-400 font-mono", "CADE_API_KEY" } " environment variable or update " code { class: "text-cyan-400 font-mono", "~/.cade/settings.json" } "." }
                    }
                }
            }
        }
    }
}

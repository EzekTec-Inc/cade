use dioxus::prelude::*;

use crate::types::{AppState, CodeLanguage};

/// Dashboard home page with feature cards and API getting-started section.
#[component]
pub fn DashboardView() -> Element {
    let state = use_context::<AppState>();
    let active_tab = use_signal(|| 0);
    let selected_lang = use_signal(|| CodeLanguage::Javascript);
    let copied_key = use_signal(|| false);
    let copied_code = use_signal(|| false);

    let (tab_title, tab_desc, tab_link, tab_href) = match active_tab() {
        0 => (
            "Send a message to an agent",
            "Make an API request to send your stateful agent a message.",
            "Get started with the API",
            "https://github.com/EzekTec-Inc/CADE/blob/main/docs/getting-started.md",
        ),
        1 => (
            "Create an agent",
            "Deploy a new autonomous agent with custom system prompts, tools, and persona.",
            "Explore agent creation",
            "https://github.com/EzekTec-Inc/CADE/blob/main/docs/agents-and-conversations.md",
        ),
        _ => (
            "Inspect agent memory",
            "Retrieve and inspect the stateful core memory blocks of an active agent.",
            "Read about memory state",
            "https://github.com/EzekTec-Inc/CADE/blob/main/docs/memory-system.md",
        ),
    };

    let code_content = code_for_tab(active_tab(), selected_lang());

    rsx! {
        // Header bar
        header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
            div {}
            div { class: "flex items-center space-x-6 text-[13px] text-gray-400 font-medium",
                span { class: "hover:text-white cursor-pointer transition duration-150", "Support" }
                span { class: "hover:text-white cursor-pointer transition duration-150", "Docs" }
                span { class: "hover:text-white cursor-pointer transition duration-150", "API reference" }
                span { class: "hover:text-white cursor-pointer transition duration-150", "Manage LLM keys" }
                span { class: "bg-[#1f222b] hover:bg-[#272a35] text-white px-3 py-1.5 rounded-md border border-[#272833] cursor-pointer text-xs font-semibold shadow transition duration-150", "Free Tier" }
            }
        }

        // Dashboard Content
        div { class: "px-10 pt-6 flex-1",
            // Greeting Heading
            h1 { class: "text-[32px] font-bold text-white mb-8 tracking-tight flex items-center space-x-2",
                span { "Evening, Stephen" }
                span { class: "inline-block animate-pulse duration-1000", "\u{1f315}" }
            }

            // Feature cards grid
            div { class: "grid grid-cols-1 md:grid-cols-3 gap-6 mb-10 select-none",
                // CARD 1: Desktop App
                feature_card_desktop { }
                // CARD 2: CADE CLI
                feature_card_cli { }
                // CARD 3: CADE API
                feature_card_api { }
            }

            // API Section
            h2 { class: "text-lg font-bold text-white mb-4 tracking-tight", "Get started with the API" }

            div { class: "border border-[#21232c] bg-[#16171d] rounded-xl overflow-hidden shadow-xl flex flex-col",
                // Tab navigation
                div { class: "px-6 py-3 border-b border-[#21232c] flex items-center justify-between select-none text-[13px] font-medium text-gray-400",
                    div { class: "flex items-center space-x-1.5",
                        tab_button { active_tab: active_tab, idx: 0, label: "Send message to an agent" }
                        tab_button { active_tab: active_tab, idx: 1, label: "Create an agent" }
                        tab_button { active_tab: active_tab, idx: 2, label: "Inspect agent memory" }
                    }
                    // API Key widget
                    api_key_widget { copied_key: copied_key, api_key: state.api_key }
                }

                // Main block split section
                div { class: "grid grid-cols-1 md:grid-cols-12 min-h-[300px]",
                    // Left column - description
                    div { class: "md:col-span-4 p-8 border-r border-[#21232c] flex flex-col justify-between",
                        div {
                            h3 { class: "text-white text-lg font-bold mb-4 tracking-tight", "{tab_title}" }
                            p { class: "text-gray-400 text-[13px] leading-6", "{tab_desc}" }
                        }
                        a {
                            href: "{tab_href}",
                            target: "_blank",
                            class: "inline-flex items-center space-x-2 text-xs font-semibold text-white border border-[#2d2f3d] bg-[#1d1e26] hover:bg-[#252735] py-2 px-4 rounded-md w-fit shadow transition duration-150",
                            span { "{tab_link}" }
                            span { class: "text-[10px]", "\u{2197}" }
                        }
                    }

                    // Right column - code display
                    code_panel {
                        selected_lang: selected_lang,
                        copied_code: copied_code,
                        code_content: code_content
                    }
                }
            }
        }
    }
}

// ── Sub-components ─────────────────────────────────────────────────────────

#[component]
fn feature_card_desktop() -> Element {
    rsx! {
        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
            div { class: "relative h-40 bg-gradient-to-br from-[#101c4c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                svg { class: "w-28 h-28 text-[#3b82f6] filter drop-shadow-[0_0_15px_rgba(59,130,246,0.3)] animate-[spin_12s_linear_infinite]", view_box: "0 0 100 100",
                    defs {
                        radialGradient { id: "sphereGrad", cx: "35%", cy: "35%", r: "65%",
                            stop { offset: "0%", "stop-color": "#60a5fa" }
                            stop { offset: "40%", "stop-color": "#2563eb" }
                            stop { offset: "100%", "stop-color": "#1e3a8a" }
                        }
                    }
                    circle { cx: "50%", cy: "50%", r: "35", fill: "url(#sphereGrad)" }
                    circle { cx: "45%", cy: "45%", r: "30", fill: "none", stroke: "rgba(255,255,255,0.08)", "stroke-width": "0.5" }
                }
                span { class: "absolute top-3 right-3 bg-[#e26a4f] text-white text-[10px] font-bold px-2 py-0.5 rounded-full uppercase tracking-wider", "New" }
            }
            div { class: "p-5 flex-1 flex flex-col justify-between",
                div {
                    h3 { class: "text-white font-bold text-[16px] mb-2 group-hover:text-[#ff7c5c] transition duration-150", "Desktop App" }
                    p { class: "text-gray-400 text-xs leading-5", "Use the CADE desktop app on macOS, Windows, or Linux" }
                }
            }
        }
    }
}

#[component]
fn feature_card_cli() -> Element {
    rsx! {
        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
            div { class: "relative h-40 bg-gradient-to-br from-[#1b343c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                svg { class: "w-24 h-24 text-teal-400 animate-[bounce_4s_ease-in-out_infinite] filter drop-shadow-[0_0_12px_rgba(20,184,166,0.35)]", view_box: "0 0 100 100",
                    rect { x: "25", y: "30", width: "50", height: "40", rx: "10", fill: "#374151" }
                    rect { x: "32", y: "36", width: "36", height: "28", rx: "5", fill: "#111827" }
                    rect { x: "37", y: "42", width: "16", height: "3", rx: "1", fill: "#10b981" }
                    rect { x: "37", y: "48", width: "24", height: "3", rx: "1", fill: "#3b82f6" }
                    rect { x: "37", y: "54", width: "12", height: "3", rx: "1", fill: "#f59e0b" }
                    circle { cx: "50", cy: "22", r: "4", fill: "#14b8a6" }
                    line { x1: "50", y1: "22", x2: "50", y2: "30", stroke: "#9ca3af", "stroke-width": "2" }
                }
            }
            div { class: "p-5 flex-1 flex flex-col justify-between",
                div {
                    h3 { class: "text-white font-bold text-[16px] mb-2 group-hover:text-[#ff7c5c] transition duration-150", "CADE CLI" }
                    p { class: "text-gray-400 text-xs leading-5", "Run memory-first CADE agents locally from your terminal" }
                }
            }
        }
    }
}

#[component]
fn feature_card_api() -> Element {
    rsx! {
        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
            div { class: "relative h-40 bg-gradient-to-br from-[#24133c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                svg { class: "w-24 h-24 filter drop-shadow-[0_0_15px_rgba(139,92,246,0.3)] animate-pulse", view_box: "0 0 100 100",
                    rect { x: "25", y: "25", width: "50", height: "50", rx: "8", fill: "#4b5563" }
                    rect { x: "30", y: "30", width: "40", height: "40", rx: "5", fill: "#1f2937" }
                    rect { x: "38", y: "38", width: "24", height: "24", rx: "3", fill: "#ff7c5c" }
                    line { x1: "20", y1: "35", x2: "25", y2: "35", stroke: "#9ca3af", "stroke-width": "2" }
                    line { x1: "20", y1: "50", x2: "25", y2: "50", stroke: "#9ca3af", "stroke-width": "2" }
                    line { x1: "20", y1: "65", x2: "25", y2: "65", stroke: "#9ca3af", "stroke-width": "2" }
                    line { x1: "75", y1: "35", x2: "80", y2: "35", stroke: "#9ca3af", "stroke-width": "2" }
                    line { x1: "75", y1: "50", x2: "80", y2: "50", stroke: "#9ca3af", "stroke-width": "2" }
                    line { x1: "75", y1: "65", x2: "80", y2: "65", stroke: "#9ca3af", "stroke-width": "2" }
                }
            }
            div { class: "p-5 flex-1 flex flex-col justify-between",
                div {
                    h3 { class: "text-white font-bold text-[16px] mb-2 group-hover:text-[#ff7c5c] transition duration-150", "CADE API" }
                    p { class: "text-gray-400 text-xs leading-5", "Build CADE agents into your applications with the API" }
                }
            }
        }
    }
}

/// A single tab button in the API getting-started section.
#[component]
fn tab_button(active_tab: Signal<i32>, idx: i32, label: String) -> Element {
    let cls = if active_tab() == idx {
        "px-4 py-2 bg-[#1d1e26] text-white rounded-md cursor-pointer border border-[#2d2f3d]"
    } else {
        "px-4 py-2 hover:text-white cursor-pointer transition duration-150"
    };

    rsx! {
        span {
            class: "{cls}",
            onclick: move |_| active_tab.set(idx),
            "{label}"
        }
    }
}

/// API key display widget with copy button.
#[component]
fn api_key_widget(copied_key: Signal<bool>, api_key: Signal<String>) -> Element {
    rsx! {
        div { class: "flex items-center space-x-2 bg-[#0f1115] border border-[#21232c] py-1.5 px-3 rounded-md text-xs",
            span { class: "text-gray-500 font-semibold", "API Key:" }
            span { class: "text-gray-300 font-mono tracking-wider",
                if api_key().len() > 8 {
                    "{&api_key()[..8]}...{&api_key()[api_key().len()-4..]}"
                } else {
                    "sk-cade-\u{2022}\u{2022}\u{2022}\u{2022}"
                }
            }
            button {
                class: "hover:text-white text-gray-500 font-semibold flex items-center space-x-1 pl-1 border-l border-[#21232c] ml-1 transition duration-150",
                onclick: move |_| {
                    copied_key.set(true);
                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(2000).await;
                        copied_key.set(false);
                    });
                },
                if copied_key() {
                    span { class: "text-emerald-400 text-[10px]", "Copied!" }
                } else {
                    svg { class: "w-4 h-4 cursor-pointer", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                        path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                    }
                }
            }
        }
    }
}

/// Code panel with language selector and syntax-highlighted blocks.
#[component]
fn code_panel(
    selected_lang: Signal<CodeLanguage>,
    copied_code: Signal<bool>,
    code_content: Vec<(&'static str, &'static str)>,
) -> Element {
    rsx! {
        div { class: "md:col-span-8 bg-[#0d0e12] p-6 flex flex-col justify-between font-mono text-[13px]",
            div { class: "flex items-center justify-between border-b border-[#1b1c24] pb-3 mb-4 select-none",
                // Language select dropdown
                div { class: "flex items-center space-x-2",
                    select {
                        class: "bg-transparent text-gray-400 border border-[#21232c] py-1 px-2.5 rounded-md cursor-pointer outline-none hover:text-white transition duration-150 text-xs",
                        onchange: move |evt| {
                            match evt.value().as_str() {
                                "python" => selected_lang.set(CodeLanguage::Python),
                                "curl" => selected_lang.set(CodeLanguage::Curl),
                                _ => selected_lang.set(CodeLanguage::Javascript),
                            }
                        },
                        option { value: "javascript", selected: selected_lang() == CodeLanguage::Javascript, "javascript" }
                        option { value: "python", selected: selected_lang() == CodeLanguage::Python, "python" }
                        option { value: "curl", selected: selected_lang() == CodeLanguage::Curl, "curl" }
                    }
                }

                // Copy button
                button {
                    class: "text-gray-500 hover:text-white transition duration-150 flex items-center space-x-1.5",
                    onclick: move |_| {
                        copied_code.set(true);
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(2000).await;
                            copied_code.set(false);
                        });
                    },
                    if copied_code() {
                        span { class: "text-emerald-400 text-xs font-semibold", "Copied!" }
                    } else {
                        svg { class: "w-4.5 h-4.5", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                            path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                        }
                    }
                }
            }

            // Rendered formatted code
            div { class: "overflow-x-auto select-all space-y-1 py-2 leading-6 text-gray-300",
                for (idx, (kw, val)) in code_content.iter().enumerate() {
                    div { class: "flex space-x-4",
                        span { class: "text-gray-600 text-right w-5 select-none", "{idx + 1}" }
                        div { class: "flex-1",
                            if !kw.is_empty() {
                                span {
                                    class: if *kw == "import" || *kw == "from" || *kw == "const" || *kw == "curl" { "text-[#ff7c5c] font-semibold mr-2" } else { "text-[#34d399] mr-2" },
                                    "{kw}"
                                }
                            }
                            if val.contains("AGENT_ID") {
                                code_agent_id { val: (*val).to_string() }
                            } else {
                                span {
                                    class: if val.starts_with('"') || val.starts_with('\'') { "text-teal-400" } else { "text-gray-300" },
                                    "{val}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Highlighted AGENT_ID placeholder in code.
#[component]
fn code_agent_id(val: String) -> Element {
    let parts: Vec<&str> = val.split("AGENT_ID").collect();
    rsx! {
        span { "{parts[0]}" }
        span { class: "bg-[#272833] border border-[#ff7c5c]/40 text-[#ff7c5c] px-1.5 py-0.5 rounded font-bold text-xs font-sans select-none", "AGENT_ID \u{2195}" }
        span { "{parts[1]}" }
    }
}

// ── Static code content ────────────────────────────────────────────────────

fn code_for_tab(tab: i32, lang: CodeLanguage) -> Vec<(&'static str, &'static str)> {
    match (tab, lang) {
        (0, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            ("const", "client = new Cade({ apiKey: \"sk-cade...OA==\" });"),
            ("", ""),
            ("const", "response = await client.agents.messages.create(\"AGENT_ID\", {"),
            ("    input:", "\"What do you remember about me?\","),
            ("});", ""),
            ("", ""),
            ("console.log", "(response.messages);"),
        ],
        (0, CodeLanguage::Python) => vec![
            ("from", "cade import Cade"),
            ("", ""),
            ("client", "= Cade(api_key=\"sk-cade...OA==\")"),
            ("", ""),
            ("response", "= client.agents.messages.create("),
            ("    agent_id=", "\"AGENT_ID\","),
            ("    input_message=", "\"What do you remember about me?\""),
            (")", ""),
            ("", ""),
            ("print", "(response.messages)"),
        ],
        (0, CodeLanguage::Curl) => vec![
            ("curl", "-X POST \"https://api.cade.ai/v1/agents/AGENT_ID/messages\" \\"),
            ("  -H", "\"Authorization: Bearer sk-cade...OA==\" \\"),
            ("  -H", "\"Content-Type: application/json\" \\"),
            ("  -d", "'{\"message\": \"What do you remember about me?\"}'"),
        ],
        (1, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            ("const", "client = new Cade({ apiKey: \"sk-cade...OA==\" });"),
            ("", ""),
            ("const", "agent = await client.agents.create({"),
            ("    name:", "\"Research-Assistant\","),
            ("    systemPrompt:", "\"You are a helpful researcher.\","),
            ("    model:", "\"gpt-4o\""),
            ("});", ""),
            ("", ""),
            ("console.log", "(\"Agent created:\", agent.id);"),
        ],
        (1, CodeLanguage::Python) => vec![
            ("from", "cade import Cade"),
            ("", ""),
            ("client", "= Cade(api_key=\"sk-cade...OA==\")"),
            ("", ""),
            ("agent", "= client.agents.create("),
            ("    name=", "\"Research-Assistant\","),
            ("    system_prompt=", "\"You are a helpful researcher.\","),
            ("    model=", "\"gpt-4o\""),
            (")", ""),
            ("", ""),
            ("print", "(f\"Agent created: {agent.id}\")"),
        ],
        (1, CodeLanguage::Curl) => vec![
            ("curl", "-X POST \"https://api.cade.ai/v1/agents\" \\"),
            ("  -H", "\"Authorization: Bearer sk-cade...OA==\" \\"),
            ("  -H", "\"Content-Type: application/json\" \\"),
            ("  -d", "'{"),
            ("    \"name\":", "\"Research-Assistant\","),
            ("    \"system_prompt\":", "\"You are a helpful researcher.\","),
            ("    \"model\":", "\"gpt-4o\""),
            ("  }'", ""),
        ],
        (2, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            ("const", "client = new Cade({ apiKey: \"sk-cade...OA==\" });"),
            ("", ""),
            ("const", "memory = await client.agents.memory.retrieve(\"AGENT_ID\");"),
            ("", ""),
            ("console.log", "(\"Core Memory Blocks:\", memory.blocks);"),
        ],
        (2, CodeLanguage::Python) => vec![
            ("from", "cade import Cade"),
            ("", ""),
            ("client", "= Cade(api_key=\"sk-cade...OA==\")"),
            ("", ""),
            ("memory", "= client.agents.memory.retrieve(agent_id=\"AGENT_ID\")"),
            ("", ""),
            ("print", "(f\"Core Memory Blocks: {memory.blocks}\")"),
        ],
        _ => vec![
            ("curl", "-X GET \"https://api.cade.ai/v1/agents/AGENT_ID/memory\" \\"),
            ("  -H", "\"Authorization: Bearer sk-cade...OA==\""),
        ],
    }
}

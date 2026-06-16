use dioxus::prelude::*;

async fn api_request(
    method: &str,
    path: &str,
    body: Option<&str>,
    api_key: &str,
) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::Cors);

    if let Some(body_str) = body {
        let js_body = wasm_bindgen::JsValue::from_str(body_str);
        opts.set_body(&js_body);
    }

    let request = Request::new_with_str_and_init(path, &opts).map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Authorization", &format!("Bearer {}", api_key))
        .map_err(|e| format!("{:?}", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{:?}", e))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{:?}", e))?;
    let resp: Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;

    if !resp.ok() {
        return Err(format!("HTTP error: {}", resp.status()));
    }

    let text_value = JsFuture::from(resp.text().map_err(|e| format!("{:?}", e))?)
        .await
        .map_err(|e| format!("{:?}", e))?;
    Ok(text_value.as_string().unwrap_or_default())
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    // Launch the Dioxus web application
    LaunchBuilder::new()
        .with_cfg(dioxus::web::Config::new().rootname("cade_gui_canvas"))
        .launch(App);
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum SelectedPage {
    Dashboard,
    Code,
    Chat,
    Agents,
    Logs,
    MemoryBlocks,
    Tools,
    Models,
    ApiKeys,
    Usage,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum CodeLanguage {
    Javascript,
    Python,
    Curl,
}

#[component]
fn App() -> Element {
    // Active states for sidebar, tabs, language, etc.
    let mut active_page = use_signal(|| SelectedPage::Dashboard);
    let mut active_tab = use_signal(|| 0); // 0 = Send message, 1 = Create agent, 2 = Inspect memory
    let mut selected_lang = use_signal(|| CodeLanguage::Javascript);
    let mut copied_key = use_signal(|| false);
    let mut copied_code = use_signal(|| false);

    // Chat specific states
    let mut selected_agent = use_signal(|| Option::<cade_api_types::AgentInfo>::None);
    let mut messages = use_signal(Vec::<cade_api_types::ChatMessage>::new);
    let mut input_text = use_signal(String::new);
    let mut is_loading = use_signal(|| false);
    let api_key = use_signal(|| {
        String::from(
            "sk-placeholder-key-for-local-development",
        )
    });
    let agent_name = selected_agent()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "deep-thought-research-agent_copy".to_string());

    use_effect(move || {
        spawn(async move {
            // First fetch the first agent
            if let Ok(agents_str) = api_request("GET", "/v1/agents", None, &api_key()).await
                && let Ok(agent_list) =
                    serde_json::from_str::<Vec<cade_api_types::AgentInfo>>(&agents_str)
                && let Some(first) = agent_list.first()
            {
                selected_agent.set(Some(first.clone()));
            }

            // Loop to poll messages
            loop {
                if let Some(agent) = selected_agent() {
                    let path = format!("/v1/agents/{}/messages", agent.id);
                    if let Ok(messages_str) = api_request("GET", &path, None, &api_key()).await
                        && let Ok(msg_list) =
                            serde_json::from_str::<Vec<cade_api_types::ChatMessage>>(&messages_str)
                    {
                        messages.set(msg_list);
                    }
                }
                gloo_timers::future::TimeoutFuture::new(1500).await;
            }
        });
    });

    // Dynamic description text and link per tab
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

    // Dynamic code blocks based on active tab and selected language
    let code_content = match (active_tab(), selected_lang()) {
        (0, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            (
                "const",
                "client = new Cade({ apiKey: \"sk-cade...OA==\" });",
            ),
            ("", ""),
            (
                "const",
                "response = await client.agents.messages.create(\"AGENT_ID\", {",
            ),
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
            (
                "curl",
                "-X POST \"https://api.cade.ai/v1/agents/AGENT_ID/messages\" \\",
            ),
            ("  -H", "\"Authorization: Bearer sk-cade...OA==\" \\"),
            ("  -H", "\"Content-Type: application/json\" \\"),
            (
                "  -d",
                "'{\"message\": \"What do you remember about me?\"}'",
            ),
        ],
        (1, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            (
                "const",
                "client = new Cade({ apiKey: \"sk-cade...OA==\" });",
            ),
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
            (
                "    \"system_prompt\":",
                "\"You are a helpful researcher.\",",
            ),
            ("    \"model\":", "\"gpt-4o\""),
            ("  }'", ""),
        ],
        (2, CodeLanguage::Javascript) => vec![
            ("import", "{ Cade } from \"@cade-ai/cade-sdk\";"),
            (
                "const",
                "client = new Cade({ apiKey: \"sk-cade...OA==\" });",
            ),
            ("", ""),
            (
                "const",
                "memory = await client.agents.memory.retrieve(\"AGENT_ID\");",
            ),
            ("", ""),
            ("console.log", "(\"Core Memory Blocks:\", memory.blocks);"),
        ],
        (2, CodeLanguage::Python) => vec![
            ("from", "cade import Cade"),
            ("", ""),
            ("client", "= Cade(api_key=\"sk-cade...OA==\")"),
            ("", ""),
            (
                "memory",
                "= client.agents.memory.retrieve(agent_id=\"AGENT_ID\")",
            ),
            ("", ""),
            ("print", "(f\"Core Memory Blocks: {memory.blocks}\")"),
        ],
        _ => vec![
            (
                "curl",
                "-X GET \"https://api.cade.ai/v1/agents/AGENT_ID/memory\" \\",
            ),
            ("  -H", "\"Authorization: Bearer sk-cade...OA==\""),
        ],
    };

    rsx! {
            // --- SIDEBAR NAVIGATION (LEFT) ---
            aside { class: "w-[240px] bg-[#0f1115] border-r border-[#1f222b] flex flex-col justify-between h-full select-none text-sm shrink-0",
                div { class: "flex flex-col",
                    // Top Brand Header
                    div { class: "p-4 flex items-center justify-between border-b border-[#1f222b]",
                        div { class: "flex items-center space-x-2",
                            // Custom SVG Brand representation
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
                                span { class: "text-gray-400 text-xs", "⊞" }
                                span { class: "font-medium text-xs text-gray-200", "Default Project" }
                            }
                            span { class: "text-gray-500 text-[10px]", "▼" }
                        }
                    }

                    // Main navigation list
                    nav { class: "px-2 space-y-0.5",
                        // Main Group
                        div { class: "text-[10px] font-bold text-gray-500 px-3 pt-3 pb-1 tracking-wider uppercase", "Dashboard" }
                        div {
                            class: if active_page() == SelectedPage::Dashboard { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Dashboard),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "🎛" }
                                span { "Dashboard" }
                            }
                        }
                        a { href: "#", class: "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer",
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "⌨" }
                                span { "Code" }
                            }
                            span { class: "text-gray-600 text-xs", "↗" }
                        }
                        div {
                            class: if active_page() == SelectedPage::Chat { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Chat),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "💬" }
                                span { "Chat" }
                            }
                        }

                        // Development Group
                        div { class: "text-[10px] font-bold text-gray-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Development" }
                        div {
                            class: if active_page() == SelectedPage::Agents { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Agents),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "🤖" }
                                span { "Agents" }
                            }
                        }
                        div {
                            class: if active_page() == SelectedPage::Logs { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Logs),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "📋" }
                                span { "Logs" }
                            }
                        }

                        // Resources Group
                        div { class: "text-[10px] font-bold text-gray-500 px-3 pt-4 pb-1 tracking-wider uppercase", "Resources" }
                        div {
                            class: if active_page() == SelectedPage::MemoryBlocks { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::MemoryBlocks),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "🧠" }
                                span { "Memory blocks" }
                            }
                        }
                        div {
                            class: if active_page() == SelectedPage::Tools { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Tools),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "🛠" }
                                span { "Tools" }
                            }
                        }
                        div {
                            class: if active_page() == SelectedPage::Models { "flex items-center justify-between px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                            onclick: move |_| active_page.set(SelectedPage::Models),
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "⚙" }
                                span { "Models" }
                            }
                        }
                        div { class: "flex items-center justify-between px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer",
                            div { class: "flex items-center space-x-2.5",
                                span { class: "text-sm", "•••" }
                                span { "More" }
                            }
                            span { class: "text-gray-600 text-[10px]", "▼" }
                        }
                    }
                }

                // Bottom controls
                div { class: "p-2 border-t border-[#1f222b] space-y-0.5",
                    div {
                        class: if active_page() == SelectedPage::ApiKeys { "flex items-center px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                        onclick: move |_| active_page.set(SelectedPage::ApiKeys),
                        span { class: "mr-2.5 text-sm", "🔑" }
                        span { "API Keys" }
                    }
                    div {
                        class: if active_page() == SelectedPage::Usage { "flex items-center px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                        onclick: move |_| active_page.set(SelectedPage::Usage),
                        span { class: "mr-2.5 text-sm", "📊" }
                        span { "Usage" }
                    }
                    div {
                        class: if active_page() == SelectedPage::Settings { "flex items-center px-3 py-2 rounded-md bg-[#16171d] text-white font-medium cursor-pointer" } else { "flex items-center px-3 py-2 rounded-md text-gray-400 hover:text-white hover:bg-[#111218] cursor-pointer" },
                        onclick: move |_| active_page.set(SelectedPage::Settings),
                        span { class: "mr-2.5 text-sm", "⚙" }
                        span { "Settings" }
                    }
                }
            }

            // --- MAIN VIEW AREA (RIGHT) ---
            main { class: "flex-1 bg-[#0f1115] overflow-y-auto flex flex-col justify-between h-full select-text pb-8",
                {
                    if active_page() == SelectedPage::Chat {
                        rsx! {
                            div { class: "flex flex-1 h-full overflow-hidden",
                            // Sub-Sidebar (Left part of Chat View)
                            div { class: "w-[260px] bg-[#16171d] border-r border-[#272833] flex flex-col p-4 justify-between h-full select-none shrink-0",
                                div { class: "flex flex-col space-y-6",
                                    // Active Agent Header
                                    div { class: "flex items-center space-x-3 p-2",
                                        div { class: "w-8 h-8 rounded-lg bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6] filter drop-shadow-[0_0_6px_rgba(236,72,153,0.3)] shrink-0" }
                                        span { class: "text-white text-sm font-semibold truncate", "{agent_name}" }
                                    }
                                    // Options Menu
                                    div { class: "flex flex-col space-y-1 text-sm text-gray-400",
                                        div { class: "flex items-center space-x-2.5 px-3 py-2 rounded-md hover:bg-[#1f212a] hover:text-white cursor-pointer transition duration-150",
                                            span { "🧠" }
                                            span { "Memory" }
                                        }
                                        div { class: "flex items-center space-x-2.5 px-3 py-2 rounded-md hover:bg-[#1f212a] hover:text-white cursor-pointer transition duration-150",
                                            span { "📝" }
                                            span { "New chat" }
                                        }
                                    }
                                    // Pinned Section
                                    div { class: "flex flex-col space-y-1",
                                        div { class: "text-[10px] font-bold text-gray-500 px-3 tracking-wider uppercase", "Pinned" }
                                        div { class: "flex items-center justify-between px-3 py-2 rounded-md bg-[#1f212a]/60 text-white font-medium cursor-pointer",
                                            div { class: "flex items-center space-x-2.5",
                                                span { "💬" }
                                                span { "Main chat" }
                                            }
                                            span { class: "text-gray-500 text-[10px]", "7mo" }
                                        }
                                    }
                                }
                                // Bottom User ID
                                div { class: "p-2 border-t border-[#272833] flex items-center space-x-2.5 select-none",
                                    div { class: "w-7 h-7 rounded-full bg-orange-500 text-white text-xs flex items-center justify-center font-bold", "SE" }
                                    span { class: "text-gray-400 text-xs", "stephen" }
                                }
                            }

                            // Main Chat Panel (Right part of Chat View)
                            div { class: "flex-1 flex flex-col justify-between bg-[#0f1115] h-full",
                                // Header bar
                                header { class: "px-6 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                                    span { class: "text-white font-medium text-sm", "Main chat" }
                                }

                                // Messages area
                                div { class: "flex-1 overflow-y-auto p-8 space-y-6 flex flex-col",
                                    if messages().is_empty() {
                                        div { class: "m-auto flex flex-col items-center select-none",
                                            div { class: "w-16 h-16 rounded-xl bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6] filter drop-shadow-[0_0_12px_rgba(236,72,153,0.4)] mb-4" }
                                            h2 { class: "text-[24px] font-semibold text-white mb-6", "Hi, I'm {agent_name}" }
                                        }
                                    } else {
                                        for m in messages().iter() {
                                            {
                                                let is_user = m.role == "user";
                                                let content_str;
                                                let content_val = if let Some(s) = m.content.as_str() {
                                                    s
                                                } else {
                                                    content_str = m.content.to_string();
                                                    &content_str
                                                };
                                                let bubble_class = if is_user {
                                                    "flex items-start space-x-3 max-w-[80%] ml-auto flex-row-reverse space-x-reverse"
                                                } else {
                                                    "flex items-start space-x-3 max-w-[80%] mr-auto"
                                                };
                                                let avatar_class = if is_user {
                                                    "w-8 h-8 rounded-lg shrink-0 flex items-center justify-center font-bold text-xs bg-orange-500 text-white"
                                                } else {
                                                    "w-8 h-8 rounded-lg shrink-0 flex items-center justify-center font-bold text-xs bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6]"
                                                };
                                                rsx! {
                                                    div { class: "{bubble_class}",
                                                        div { class: "{avatar_class}",
                                                            if is_user { "U" } else { "AI" }
                                                        }
                                                        div { class: "flex flex-col bg-[#16171d]/60 border border-[#272833] p-4 rounded-xl text-sm",
                                                            div { class: "text-[10px] font-bold text-gray-500 uppercase select-none mb-1", if is_user { "user" } else { "assistant" } }
                                                            p { class: "text-gray-200 mt-1 whitespace-pre-wrap", "{content_val}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Message input section
                                div { class: "p-6 bg-[#0f1115] border-t border-[#111218]",
                                    div { class: "relative border border-[#272833] bg-[#16171d] rounded-xl p-4 flex flex-col space-y-2",
                                        textarea {
                                            class: "bg-transparent text-gray-200 placeholder-gray-500 outline-none w-full text-sm resize-none h-12",
                                            placeholder: "Ask anything, @ to add files, / for commands",
                                            value: "{input_text}",
                                            oninput: move |e| input_text.set(e.value().clone()),
                                            onkeydown: move |e| {
                                                if e.key() == Key::Enter && !e.modifiers().shift() {
                                                    e.stop_propagation();
                                                    let text = input_text().trim().to_string();
                                                    if !text.is_empty() && !is_loading() {
                                                        is_loading.set(true);
                                                        input_text.set(String::new());

                                                        // Optimistically insert user message
                                                        let mut current_msgs = messages();
                                                        current_msgs.push(cade_api_types::ChatMessage {
                                                            id: format!("temp-{}", messages().len()),
                                                            role: "user".to_string(),
                                                            content: serde_json::Value::String(text.clone()),
                                                            conversation_id: None,
                                                        });
                                                        messages.set(current_msgs);

                                                        let agent_id = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
                                                        let key = api_key();

                                                        spawn(async move {
                                                            let path = format!("/v1/agents/{}/run", agent_id);
                                                            let body = serde_json::json!({ "input": text });
                                                            let _ = api_request("POST", &path, Some(&body.to_string()), &key).await;
                                                            is_loading.set(false);
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                        div { class: "flex items-center justify-between pt-2 border-t border-[#272833]/40 select-none",
                                            div { class: "flex items-center space-x-3 text-xs text-gray-500 font-medium",
                                                span { class: "flex items-center space-x-1",
                                                    span { class: "text-emerald-500", "🟢" }
                                                    span { "Cloud" }
                                                }
                                                span { class: "flex items-center space-x-1",
                                                    span { "📁" }
                                                    span { "root" }
                                                }
                                            }
                                            button {
                                                class: if is_loading() { "w-7 h-7 bg-[#ff7c5c] text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150 opacity-50 cursor-not-allowed" } else { "w-7 h-7 bg-[#ff7c5c] text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150" },
                                                onclick: move |_| {
                                                    let text = input_text().trim().to_string();
                                                    if !text.is_empty() && !is_loading() {
                                                        is_loading.set(true);
                                                        input_text.set(String::new());

                                                        let mut current_msgs = messages();
                                                        current_msgs.push(cade_api_types::ChatMessage {
                                                            id: format!("temp-{}", messages().len()),
                                                            role: "user".to_string(),
                                                            content: serde_json::Value::String(text.clone()),
                                                            conversation_id: None,
                                                        });
                                                        messages.set(current_msgs);

                                                        let agent_id = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
                                                        let key = api_key();

                                                        spawn(async move {
                                                            let path = format!("/v1/agents/{}/run", agent_id);
                                                            let body = serde_json::json!({ "input": text });
                                                            let _ = api_request("POST", &path, Some(&body.to_string()), &key).await;
                                                            is_loading.set(false);
                                                        });
                                                    }
                                                },
                                                // Send SVG icon
                                                svg { class: "w-4 h-4 transform rotate-90", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2.5",
                                                    path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M12 19V5m-7 7l7-7 7 7" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    } else {
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
                        span { class: "inline-block animate-pulse duration-1000", "🌕" }
                    }

                    // Grid containing the 3 beautifully formatted feature cards
                    div { class: "grid grid-cols-1 md:grid-cols-3 gap-6 mb-10 select-none",
                        // CARD 1: Desktop App
                        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
                            div { class: "relative h-40 bg-gradient-to-br from-[#101c4c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                                // Dynamic 3D Sphere SVG
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
                                // "New" Badge overlay
                                span { class: "absolute top-3 right-3 bg-[#e26a4f] text-white text-[10px] font-bold px-2 py-0.5 rounded-full uppercase tracking-wider", "New" }
                            }
                            div { class: "p-5 flex-1 flex flex-col justify-between",
                                div {
                                    h3 { class: "text-white font-bold text-[16px] mb-2 group-hover:text-[#ff7c5c] transition duration-150", "Desktop App" }
                                    p { class: "text-gray-400 text-xs leading-5", "Use the CADE desktop app on macOS, Windows, or Linux" }
                                }
                            }
                        }

                        // CARD 2: CADE CLI
                        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
                            div { class: "relative h-40 bg-gradient-to-br from-[#1b343c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                                // 3D Terminal Robot representation using SVGs
                                svg { class: "w-24 h-24 text-teal-400 animate-[bounce_4s_ease-in-out_infinite] filter drop-shadow-[0_0_12px_rgba(20,184,166,0.35)]", view_box: "0 0 100 100",
                                    // Robot Body
                                    rect { x: "25", y: "30", width: "50", height: "40", rx: "10", fill: "#374151" }
                                    // Terminal screen
                                    rect { x: "32", y: "36", width: "36", height: "28", rx: "5", fill: "#111827" }
                                    // Code line text simulations
                                    rect { x: "37", y: "42", width: "16", height: "3", rx: "1", fill: "#10b981" }
                                    rect { x: "37", y: "48", width: "24", height: "3", rx: "1", fill: "#3b82f6" }
                                    rect { x: "37", y: "54", width: "12", height: "3", rx: "1", fill: "#f59e0b" }
                                    // Tiny robot eyes/antenna
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

                        // CARD 3: CADE API
                        div { class: "bg-[#16171d] border border-[#21232c] rounded-xl overflow-hidden hover:border-[#ff7c5c] group transition duration-300 flex flex-col justify-between shadow-lg",
                            div { class: "relative h-40 bg-gradient-to-br from-[#24133c] via-[#12131d] to-[#12131c] flex items-center justify-center p-4 overflow-hidden",
                                // Custom Metallic frame/core chip SVG
                                svg { class: "w-24 h-24 filter drop-shadow-[0_0_15px_rgba(139,92,246,0.3)] animate-pulse", view_box: "0 0 100 100",
                                    rect { x: "25", y: "25", width: "50", height: "50", rx: "8", fill: "#4b5563" }
                                    rect { x: "30", y: "30", width: "40", height: "40", rx: "5", fill: "#1f2937" }
                                    rect { x: "38", y: "38", width: "24", height: "24", rx: "3", fill: "#ff7c5c" }
                                    // Connectors
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

                    // API Section heading
                    h2 { class: "text-lg font-bold text-white mb-4 tracking-tight", "Get started with the API" }

                    // API block section container
                    div { class: "border border-[#21232c] bg-[#16171d] rounded-xl overflow-hidden shadow-xl flex flex-col",
                        // Tab navigation subheader
                        div { class: "px-6 py-3 border-b border-[#21232c] flex items-center justify-between select-none text-[13px] font-medium text-gray-400",
                            div { class: "flex items-center space-x-1.5",
                                span {
                                    class: if active_tab() == 0 { "px-4 py-2 bg-[#1d1e26] text-white rounded-md cursor-pointer border border-[#2d2f3d]" } else { "px-4 py-2 hover:text-white cursor-pointer transition duration-150" },
                                    onclick: move |_| active_tab.set(0),
                                    "Send message to an agent"
                                }
                                span {
                                    class: if active_tab() == 1 { "px-4 py-2 bg-[#1d1e26] text-white rounded-md cursor-pointer border border-[#2d2f3d]" } else { "px-4 py-2 hover:text-white cursor-pointer transition duration-150" },
                                    onclick: move |_| active_tab.set(1),
                                    "Create an agent"
                                }
                                span {
                                    class: if active_tab() == 2 { "px-4 py-2 bg-[#1d1e26] text-white rounded-md cursor-pointer border border-[#2d2f3d]" } else { "px-4 py-2 hover:text-white cursor-pointer transition duration-150" },
                                    onclick: move |_| active_tab.set(2),
                                    "Inspect agent memory"
                                }
                            }

                            // Right side API Key widget
                            div { class: "flex items-center space-x-2 bg-[#0f1115] border border-[#21232c] py-1.5 px-3 rounded-md text-xs",
                                span { class: "text-gray-500 font-semibold", "API Key:" }
                                span { class: "text-gray-300 font-mono tracking-wider", "sk-cade-••••••••••••••••••••••••••••••••" }
                                button {
                                    class: "hover:text-white text-gray-500 font-semibold flex items-center space-x-1 pl-1 border-l border-[#21232c] ml-1 transition duration-150",
                                    onclick: move |_| {
                                        copied_key.set(true);
                                        // Reset badge in 2 seconds natively on WASM
                                        spawn(async move {
                                            gloo_timers::future::TimeoutFuture::new(2000).await;
                                            copied_key.set(false);
                                        });
                                    },
                                    if copied_key() {
                                        span { class: "text-emerald-400 text-[10px]", "Copied!" }
                                    } else {
                                        // Custom copy SVG icon
                                        svg { class: "w-4 h-4 cursor-pointer", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                            path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                                        }
                                    }
                                }
                            }
                        }

                        // Main block split section
                        div { class: "grid grid-cols-1 md:grid-cols-12 min-h-[300px]",
                            // Left column
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
                                    span { class: "text-[10px]", "↗" }
                                }
                            }

                            // Right column (Codeblock displayer)
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

                                    // Code copy button
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
                                            // Custom copy SVG icon
                                            svg { class: "w-4.5 h-4.5", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                                path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                                            }
                                        }
                                    }
                                }

                                // Rendered formatted code with styled syntax highlighting
                                div { class: "overflow-x-auto select-all space-y-1 py-2 leading-6 text-gray-300",
                                    for (idx, (kw, val)) in code_content.iter().enumerate() {
                                        div { class: "flex space-x-4",
                                            // Line numbers
                                            span { class: "text-gray-600 text-right w-5 select-none", "{idx + 1}" }
                                            // Colored keywords & values
                                            div { class: "flex-1",
                                                if !kw.is_empty() {
                                                    span {
                                                        class: if *kw == "import" || *kw == "from" || *kw == "const" || *kw == "curl" { "text-[#ff7c5c] font-semibold mr-2" } else { "text-[#34d399] mr-2" },
                                                        "{kw}"
                                                    }
                                                }
                                                // Handle special styling for AGENT_ID tag
                                                if val.contains("AGENT_ID") {
                                                    {
                                                        let parts: Vec<&str> = val.split("AGENT_ID").collect();
                                                        rsx! {
                                                            span { "{parts[0]}" }
                                                            span { class: "bg-[#272833] border border-[#ff7c5c]/40 text-[#ff7c5c] px-1.5 py-0.5 rounded font-bold text-xs font-sans select-none", "AGENT_ID ↕" }
                                                            span { "{parts[1]}" }
                                                        }
                                                    }
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
                }
            }
        }
    }
    }
    }
}

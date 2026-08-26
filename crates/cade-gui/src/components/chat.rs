use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

/// Full chat view with message history and input area.
#[component]
pub fn ChatView() -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let agent_name = (state.selected_agent)()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "deep-thought-research-agent_copy".to_string());

    // Load messages when the active conversation or selected agent changes.
    // This replaces the old background-polling approach which would overwrite
    // streaming content mid-stream.
    use_effect(move || {
        let conv_id = (state.active_conversation)();
        let agent_id = (state.selected_agent)()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let api_client = client();
        let mut msgs = state.messages;
        let mut active_stream = state.active_stream;

        // Abort the previous stream on conversation/agent switch
        active_stream
            .peek()
            .0
            .store(true, std::sync::atomic::Ordering::Release);
        active_stream.set(crate::types::SafeAbortHandle::default());

        spawn(async move {
            if !agent_id.is_empty()
                && let Ok(list) = api_client.get_messages(&agent_id, conv_id.as_deref()).await
            {
                msgs.set(list);
            }
        });
    });

    rsx! {
        div { class: "flex flex-1 h-full overflow-hidden",
            chat_sidebar {
                agent_name: agent_name.clone(),
                conversations: state.conversations,
                active_conversation: state.active_conversation,
                selected_agent: state.selected_agent,
                api_key: state.api_key,
            }

            div { class: "flex-1 flex flex-col justify-between bg-[#040711] h-full",
                header { class: "px-6 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70",
                    span { class: "text-white font-medium text-sm", "Main chat" }
                }

                messages_panel { messages: state.messages, agent_name: agent_name.clone() }

                input_area {
                    input_text: state.input_text,
                    is_loading: state.is_loading,
                    messages: state.messages,
                    selected_agent: state.selected_agent,
                    api_key: state.api_key,
                    active_conversation: state.active_conversation,
                }
            }
        }
    }
}

// ── Chat sidebar with conversation management ────────────────────────────

#[component]
fn chat_sidebar(
    agent_name: String,
    conversations: Signal<Vec<cade_api_types::ConversationInfo>>,
    active_conversation: Signal<Option<String>>,
    selected_agent: Signal<Option<cade_api_types::AgentInfo>>,
    api_key: Signal<String>,
) -> Element {
    let state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let mut show_new = use_signal(|| false);
    let mut new_title = use_signal(String::new);

    let checkpoints = use_signal(Vec::<serde_json::Value>::new);
    let agent_id_for_cp = selected_agent().map(|a| a.id.clone()).unwrap_or_default();

    let cp_api_client = client;
    use_effect(move || {
        let a_id = agent_id_for_cp.clone();
        let api = cp_api_client;
        let mut cps = checkpoints;
        spawn(async move {
            if !a_id.is_empty()
                && let Ok(data) = api().list_checkpoints(&a_id).await
            {
                cps.set(data);
            }
        });
    });

    let agent_id_for_convs = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
    let conv_api_client = client;
    use_effect(move || {
        let a_id = agent_id_for_convs.clone();
        let api = conv_api_client;
        let mut conv_sig = conversations;
        spawn(async move {
            if !a_id.is_empty()
                && let Ok(mut data) = api().list_conversations(&a_id).await
            {
                data.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
                conv_sig.set(data);
            }
        });
    });

    let mut create_conv = move || {
        let title = new_title().trim().to_string();
        if title.is_empty() {
            return;
        }
        let agent_id = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
        let api_client = client();
        let mut convs = conversations;
        let mut active = active_conversation;
        spawn(async move {
            match api_client
                .create_conversation(&agent_id, Some(&title))
                .await
            {
                Ok(conv) => {
                    let mut list = convs();
                    list.push(conv.clone());
                    convs.set(list);
                    active.set(Some(conv.id));
                    add_toast(&state, ToastLevel::Success, "Conversation created", &title);
                }
                Err(e) => add_toast(
                    &state,
                    ToastLevel::Error,
                    "Failed to create conversation",
                    e,
                ),
            }
        });
        new_title.set(String::new());
        show_new.set(false);
    };

    let delete_conv = move |conv_id: String| {
        let agent_id = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
        let api_client = client();
        let mut convs = conversations;
        let mut active = active_conversation;
        spawn(async move {
            match api_client.delete_conversation(&agent_id, &conv_id).await {
                Ok(_) => {
                    let mut list = convs();
                    list.retain(|c| c.id != conv_id);
                    convs.set(list);
                    if active() == Some(conv_id.clone()) {
                        active.set(None);
                    }
                    add_toast(&state, ToastLevel::Success, "Conversation deleted", "");
                }
                Err(e) => add_toast(
                    &state,
                    ToastLevel::Error,
                    "Failed to delete conversation",
                    e,
                ),
            }
        });
    };

    let current_title = active_conversation()
        .and_then(|id| {
            conversations()
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.title.clone())
        })
        .unwrap_or_else(|| "All messages".to_string());

    // Pre-compute conversation rows outside RSX to avoid let-bindings in for-body
    let conv_rows: Vec<(String, String, String, i64, bool)> = conversations()
        .iter()
        .map(|conv| {
            let is_active = active_conversation() == Some(conv.id.clone());
            let date_str = if conv.updated_at > 0 {
                chrono::DateTime::from_timestamp(conv.updated_at, 0)
                    .map(|dt| dt.format("%m/%d %H:%M").to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            (conv.id.clone(), conv.title.clone(), date_str, conv.message_count, is_active)
        })
        .collect();

    rsx! {
        div { class: "w-[260px] bg-[#090d16] border-r border-[#1e293b] flex flex-col p-4 justify-between h-full select-none shrink-0",
            div { class: "flex flex-col space-y-6",
                div { class: "flex items-center space-x-3 p-2",
                    div { class: "w-8 h-8 rounded-lg bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6] filter drop-shadow-[0_0_6px_rgba(236,72,153,0.3)] shrink-0" }
                    span { class: "text-white text-sm font-semibold truncate", "{agent_name}" }
                }

                div { class: "flex flex-col space-y-1 text-sm text-gray-400",
                    div {
                        class: "flex items-center space-x-2.5 px-3 py-2 rounded-md hover:bg-[#1f212a] hover:text-white cursor-pointer transition duration-150",
                        onclick: move |_| show_new.set(!show_new()),
                        span { "\u{1f4dd}" }
                        span { "New chat" }
                    }
                    if show_new() {
                        div { class: "flex flex-col space-y-2 px-3 pb-2",
                            input {
                                class: "bg-[#1f212a] text-white text-xs rounded-md px-2 py-1.5 outline-none border border-[#1e293b]",
                                placeholder: "Conversation title",
                                value: "{new_title}",
                                oninput: move |e| new_title.set(e.value().clone()),
                                onkeydown: move |e| {
                                    if e.key() == Key::Enter {
                                        create_conv();
                                    }
                                }
                            }
                            button {
                                class: "text-xs bg-sky-500 text-white rounded-md px-2 py-1.5 hover:bg-[#e26a4f] transition",
                                onclick: move |_| create_conv(),
                                "Create"
                            }
                        }
                    }
                }

                div { class: "flex flex-col space-y-1",
                    div { class: "text-[10px] font-bold text-gray-500 px-3 tracking-wider uppercase mb-1", "Conversations" }
                    // "All messages" — shows all messages for the agent
                    div {
                        class: if active_conversation().is_none() {
                            "flex items-center justify-between px-3 py-2 rounded-md bg-[#1f212a] text-white font-medium cursor-pointer"
                        } else {
                            "flex items-center justify-between px-3 py-2 rounded-md hover:bg-[#1f212a]/60 text-gray-400 cursor-pointer"
                        },
                        onclick: move |_| active_conversation.set(None),
                        div { class: "flex items-center space-x-2.5",
                            span { "\u{1f4ac}" }
                            span { "All messages" }
                        }
                    }
                    {conv_rows.iter().map(|(conv_id, conv_title, date_str, msg_count, is_active)| {
                        let id_sel = conv_id.clone();
                        let id_del = conv_id.clone();
                        let title = conv_title.clone();
                        let date_label = date_str.clone();
                        let count = *msg_count;
                        let is_active = *is_active;
                        let del = delete_conv;
                        rsx! {
                            div {
                                key: "{id_sel}",
                                class: if is_active {
                                    "flex items-center justify-between px-3 py-2 rounded-md bg-[#1f212a] text-white font-medium cursor-pointer"
                                } else {
                                    "flex items-center justify-between px-3 py-2 rounded-md hover:bg-[#1f212a]/60 text-gray-400 cursor-pointer transition duration-100"
                                },
                                onclick: move |_| active_conversation.set(Some(id_sel.clone())),
                                div { class: "flex flex-col min-w-0 pr-2 truncate",
                                    div { class: "flex items-center space-x-2 truncate",
                                        span { class: "text-xs shrink-0 select-none", "💬" }
                                        span { class: "text-xs font-medium truncate", "{title}" }
                                    }
                                    div { class: "flex items-center space-x-1.5 text-[10px] text-gray-500 pl-5 mt-0.5 select-none",
                                        if count > 0 {
                                            span { "{count} msgs" }
                                        }
                                        if !date_label.is_empty() {
                                            span { "· {date_label}" }
                                        }
                                    }
                                }
                                button {
                                    class: "text-gray-600 hover:text-red-400 text-xs shrink-0 ml-1 p-1 rounded hover:bg-red-500/10 transition",
                                    title: "Delete conversation",
                                    onclick: move |e| {
                                        e.stop_propagation();
                                        del(id_del.clone());
                                    },
                                    "✖"
                                }
                            }
                        }
                    })}
                }

                // Checkpoints Timeline List
                div { class: "flex flex-col space-y-1 pt-4 border-t border-[#1e293b]/30 overflow-y-auto max-h-[180px] hide-scrollbar",
                    div { class: "text-[10px] font-bold text-gray-500 px-3 tracking-wider uppercase mb-1.5", "Checkpoints Timeline" }
                    if checkpoints().is_empty() {
                        div { class: "text-[10px] text-gray-500 px-3 italic select-none", "No checkpoints recorded." }
                    } else {
                        {checkpoints().into_iter().map(|cp| {
                            let cp_id = cp.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            let cp_label = cp.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let cp_desc = cp.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let title = if cp_label.is_empty() { cp_id.clone() } else { cp_label };
                            let agent_id_act = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
                            let cp_id_act = cp_id.clone();
                            let api = client;
                            let st_toast = state;
                            rsx! {
                                div { class: "group flex flex-col p-2 rounded-md hover:bg-[#1f212a]/40 text-gray-400 hover:text-white transition duration-150 relative",
                                    div { class: "flex items-start justify-between min-w-0 gap-2",
                                        div { class: "flex items-center space-x-2 min-w-0",
                                            span { class: "text-xs select-none", "⏱" }
                                            span { class: "text-xs font-medium truncate", "{title}" }
                                        }
                                        button {
                                            class: "text-[9px] bg-sky-500/10 text-sky-400 hover:bg-sky-500/20 border border-[#ff7c5c]/10 rounded px-1.5 py-0.5 opacity-0 group-hover:opacity-100 transition shrink-0 select-none",
                                            onclick: move |e| {
                                                e.stop_propagation();
                                                let api_act = api();
                                                let a_id = agent_id_act.clone();
                                                let c_id = cp_id_act.clone();
                                                spawn(async move {
                                                    match api_act.restore_checkpoint(&a_id, &c_id).await {
                                                        Ok(_) => add_toast(&st_toast, ToastLevel::Success, "Fork requested", format!("Forking checkpoint {c_id}")),
                                                        Err(e) => add_toast(&st_toast, ToastLevel::Error, "Fork failed", e),
                                                    }
                                                });
                                            },
                                            "Fork"
                                        }
                                    }
                                    if !cp_desc.is_empty() {
                                        div { class: "text-[9px] text-gray-500 mt-0.5 line-clamp-1 truncate font-medium", "{cp_desc}" }
                                    }
                                }
                            }
                        })}
                    }
                }
            }

            div { class: "p-2 border-t border-[#1e293b] flex items-center space-x-2.5 select-none",
                div { class: "w-7 h-7 rounded-full bg-orange-500 text-white text-xs flex items-center justify-center font-bold", "SE" }
                span { class: "text-gray-400 text-xs", "{current_title}" }
            }
        }
    }
}

// ── Messages panel ───────────────────────────────────────────────────────

#[component]
fn messages_panel(
    messages: Signal<Vec<cade_api_types::ChatMessage>>,
    agent_name: String,
) -> Element {
    // Auto-scroll to bottom when messages change
    use_effect(move || {
        let _ = messages();
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Some(el) = doc.get_element_by_id("chat-messages-panel")
        {
            el.set_scroll_top(el.scroll_height());
        }
    });

    rsx! {
        div {
            id: "chat-messages-panel",
            class: "flex-1 overflow-y-auto p-8 space-y-6 flex flex-col",
            if messages().is_empty() {
                div { class: "m-auto flex flex-col items-center select-none",
                    div { class: "w-16 h-16 rounded-xl bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6] filter drop-shadow-[0_0_12px_rgba(236,72,153,0.4)] mb-4" }
                    h2 { class: "text-[24px] font-semibold text-white mb-6", "Hi, I'm {agent_name}" }
                }
            } else {
                for m in messages().iter() {
                    message_bubble { key: "{m.id}", id: m.id.clone() }
                }
            }
        }
    }
}

// ── Message bubble with rich rendering ───────────────────────────────────

/// Split text into (reasoning, content) if `<reasoning>...</reasoning>` tags
/// are present. Otherwise returns `None`.
fn split_reasoning(text: &str) -> Option<(String, String)> {
    let start_tag = "<reasoning>";
    let end_tag = "</reasoning>";
    let start = text.find(start_tag)?;
    let end = text.find(end_tag)?;
    let reasoning = text[start + start_tag.len()..end].trim().to_string();
    let content = format!("{}{}", &text[..start], &text[end + end_tag.len()..])
        .trim()
        .to_string();
    Some((reasoning, content))
}

#[component]
fn message_bubble(id: String) -> Element {
    let mut state = use_context::<AppState>();
    let messages = state.messages.read();
    let message = match messages.iter().find(|m| m.id == id) {
        Some(m) => m,
        None => return rsx! {},
    };

    let is_user = message.role == "user";
    let is_tool = message.role == "tool";
    let is_streaming = message.id.starts_with("streaming-");

    let bubble_class = if is_user {
        "flex items-start space-x-3 max-w-[80%] ml-auto flex-row-reverse space-x-reverse"
    } else {
        "flex items-start space-x-3 max-w-[80%] mr-auto"
    };

    let avatar_class = if is_user {
        "w-8 h-8 rounded-lg shrink-0 flex items-center justify-center font-bold text-xs bg-orange-500 text-white"
    } else if is_tool {
        "w-8 h-8 rounded-lg shrink-0 flex items-center justify-center font-bold text-xs bg-gray-600 text-white"
    } else {
        "w-8 h-8 rounded-lg shrink-0 flex items-center justify-center font-bold text-xs bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6]"
    };

    let avatar_label = if is_user {
        "U"
    } else if is_tool {
        "\u{2699}"
    } else {
        "AI"
    };

    let role_label = if is_user {
        "user"
    } else if is_tool {
        "tool"
    } else if is_streaming {
        "assistant (streaming\u{2026})"
    } else {
        "assistant"
    };

    if is_tool {
        let tool_name = message
            .content
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let result_content = message
            .content
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_error = message
            .content
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let border_class = if is_error {
            "border-red-500/40"
        } else {
            "border-[#373840]"
        };

        rsx! {
            div { class: "{bubble_class}",
                div { class: "{avatar_class}", "{avatar_label}" }
                div { class: "flex flex-col bg-[#1a1d24] border {border_class} p-4 rounded-xl text-sm group relative",
                    div { class: "text-[10px] font-bold text-gray-500 uppercase select-none mb-1",
                        "tool \u{2014} {tool_name}"
                    }
                    p { class: "text-gray-300 mt-1 whitespace-pre-wrap text-xs font-mono", "{result_content}" }
                    div { class: "absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition duration-150 select-none",
                        button {
                            class: "text-gray-500 hover:text-white transition duration-150 p-1 bg-[#1e2029] border border-[#2d313f] rounded",
                            title: "Copy content",
                            onclick: {
                                let text = result_content.to_string();
                                move |_| {
                                    crate::api::copyText(&text);
                                }
                            },
                            svg { class: "w-3.5 h-3.5", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                            }
                        }
                    }
                }
            }
        }
    } else if is_user {
        let content_str;
        let content_val = if let Some(s) = message.content.as_str() {
            s
        } else {
            content_str = message.content.to_string();
            &content_str
        };

        rsx! {
            div { class: "{bubble_class}",
                div { class: "{avatar_class}", "{avatar_label}" }
                div { class: "flex flex-col bg-[#090d16]/60 border border-[#1e293b] p-4 rounded-xl text-sm group relative",
                    div { class: "text-[10px] font-bold text-gray-500 uppercase select-none mb-1", "{role_label}" }
                    p { class: "text-gray-200 mt-1 whitespace-pre-wrap break-words", "{content_val}" }
                    div { class: "absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition duration-150 select-none",
                        button {
                            class: "text-gray-500 hover:text-white transition duration-150 p-1 bg-[#1e2029] border border-[#2d313f] rounded",
                            title: "Copy content",
                            onclick: {
                                let text = content_val.to_string();
                                move |_| {
                                    crate::api::copyText(&text);
                                }
                            },
                            svg { class: "w-3.5 h-3.5", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                            }
                        }
                    }
                }
            }
        }
    } else {
        let content_str;
        let content_val = if let Some(s) = message.content.as_str() {
            s
        } else {
            content_str = message.content.to_string();
            &content_str
        };

        let (display_text, reasoning_val) = {
            let mut cache = state.parsed_messages.write();
            if let Some(cached) = cache.get(&message.id) {
                cached.clone()
            } else {
                let reasoning_parts = split_reasoning(content_val);
                let (reason, disp) = if let Some((ref r, ref d)) = reasoning_parts {
                    (Some(r.clone()), d.clone())
                } else {
                    (None, content_val.to_string())
                };
                if !is_streaming {
                    cache.insert(message.id.clone(), (disp.clone(), reason.clone()));
                }
                (disp, reason)
            }
        };

        rsx! {
            div { class: "{bubble_class}",
                div { class: "{avatar_class}", "{avatar_label}" }
                div { class: "flex flex-col bg-[#090d16]/60 border border-[#1e293b] p-4 rounded-xl text-sm group relative",
                    div { class: "text-[10px] font-bold text-gray-500 uppercase select-none mb-1", "{role_label}" }
                    if let Some(ref reasoning) = reasoning_val {
                        details { class: "mb-2",
                            summary { class: "text-yellow-500 text-xs cursor-pointer hover:text-yellow-400 select-none",
                                "\u{1f4ad} Reasoning"
                            }
                            p { class: "text-gray-400 mt-1 whitespace-pre-wrap text-xs italic border-l-2 border-yellow-500/30 pl-2", "{reasoning}" }
                        }
                    }
                    div { class: "relative",
                        crate::components::markdown::MarkdownView { content: display_text }
                        if is_streaming {
                            span { class: "animate-pulse text-gray-500 font-mono inline-block ml-1", "\u{2502}" }
                        }
                    }
                    div { class: "absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition duration-150 select-none",
                        button {
                            class: "text-gray-500 hover:text-white transition duration-150 p-1 bg-[#1e2029] border border-[#2d313f] rounded",
                            title: "Copy content",
                            onclick: {
                                let text = content_val.to_string();
                                move |_| {
                                    crate::api::copyText(&text);
                                }
                            },
                            svg { class: "w-3.5 h-3.5", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Input area ───────────────────────────────────────────────────────────

#[component]
fn input_area(
    input_text: Signal<String>,
    is_loading: Signal<bool>,
    messages: Signal<Vec<cade_api_types::ChatMessage>>,
    selected_agent: Signal<Option<cade_api_types::AgentInfo>>,
    api_key: Signal<String>,
    active_conversation: Signal<Option<String>>,
) -> Element {
    let mut state = use_context::<AppState>();
    let client = use_context::<Memo<crate::api::CadeApiClient>>();

    let mut show_suggestions = use_signal(|| false);
    let mut suggestions = use_signal(Vec::<String>::new);
    let mut active_index = use_signal(|| 0usize);

    let mut do_send = move || {
        let text = input_text().trim().to_string();
        if text.is_empty() || is_loading() {
            return;
        }
        input_text.set(String::new());
        show_suggestions.set(false);

        // Abort controller setup for the active stream (safe atomic bool cancel token)
        let cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        state
            .active_stream
            .set(crate::types::SafeAbortHandle(cancel_token.clone()));

        let agent_id = selected_agent().map(|a| a.id.clone()).unwrap_or_default();
        let api_client = client();
        let conv_id = active_conversation();
        let coordinator =
            crate::chat_session::ChatSessionCoordinator::new(api_client.clone(), agent_id.clone(), conv_id);

        let state_toast = state;
        let mut convs = state.conversations;
        let mut active_conv = active_conversation;
        let agent_id_clone = agent_id.clone();
        let api_client_clone = api_client.clone();

        spawn(async move {
            let result = coordinator
                .dispatch_turn(&text, messages, is_loading, cancel_token)
                .await;

            match result {
                Ok(crate::chat_session::ChatTurnOutcome::Completed { assigned_conversation_id, .. }) => {
                    if active_conv().is_none() && let Some(cid) = assigned_conversation_id {
                        active_conv.set(Some(cid));
                        if let Ok(mut list) = api_client_clone.list_conversations(&agent_id_clone).await {
                            list.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
                            convs.set(list);
                        }
                    }
                }
                Err(e) => {
                    add_toast(&state_toast, ToastLevel::Error, "Stream failed", e);
                }
                _ => {}
            }
        });
    };

    rsx! {
        div { class: "p-6 bg-[#040711] border-t border-[#1e293b]/70",
            div { class: "relative border border-[#1e293b] bg-[#090d16] rounded-xl p-4 flex flex-col space-y-2",
                if show_suggestions() && !suggestions().is_empty() {
                    div { class: "absolute bottom-full left-0 right-0 mb-2 bg-[#090d16] border border-[#1e293b] rounded-xl overflow-hidden shadow-2xl z-50 max-h-48 overflow-y-auto select-none",
                        {suggestions().into_iter().enumerate().map(|(idx, s)| {
                            let is_active = active_index() == idx;
                            let row_class = if is_active {
                                "px-4 py-2 bg-sky-500/10 text-white font-medium text-xs cursor-pointer flex items-center justify-between"
                            } else {
                                "px-4 py-2 hover:bg-[#1f212a] text-gray-400 text-xs cursor-pointer flex items-center justify-between"
                            };
                            let s_click = s.clone();
                            rsx! {
                                div {
                                    key: "{s_click}",
                                    class: "{row_class}",
                                    onclick: move |_| {
                                        let text = input_text();
                                        if text.starts_with('/') {
                                            input_text.set(format!("{} ", s_click));
                                        } else if let Some(pos) = text.rfind('@') {
                                            let prefix = &text[..pos];
                                            input_text.set(format!("{}@{} ", prefix, s_click));
                                        }
                                        show_suggestions.set(false);
                                    },
                                    div { class: "flex items-center space-x-2",
                                        span { class: "text-gray-500 font-mono", if s_click.starts_with('/') { "⚡" } else { "📄" } }
                                        span { class: "font-mono", "{s_click}" }
                                    }
                                    if is_active {
                                        span { class: "text-[10px] text-gray-500 font-semibold uppercase tracking-wider", "Enter to select" }
                                    }
                                }
                            }
                        })}
                    }
                }
                textarea {
                    class: "bg-transparent text-gray-200 placeholder-gray-500 outline-none w-full text-sm resize-none h-12",
                    placeholder: "Ask anything, @ to add files, / for commands",
                    value: "{input_text}",
                    prevent_default: "onkeydown",
                    oninput: move |e| {
                        let val = e.value();
                        input_text.set(val.clone());
                        if let Some(query) = val.strip_prefix('/') {
                            let cmds = vec![
                                "/help".to_string(),
                                "/memory".to_string(),
                                "/clear_context".to_string(),
                                "/compact".to_string(),
                                "/approvals".to_string(),
                                "/settings".to_string(),
                            ];
                            let filtered: Vec<String> = cmds.into_iter().filter(|c| c.contains(query)).collect();
                            if !filtered.is_empty() {
                                suggestions.set(filtered);
                                show_suggestions.set(true);
                                active_index.set(0);
                            } else {
                                show_suggestions.set(false);
                            }
                        } else if let Some(pos) = val.rfind('@') {
                            let query = &val[pos+1..];
                            let files = vec![
                                "src/main.rs".to_string(),
                                "Cargo.toml".to_string(),
                                "README.md".to_string(),
                                "CONTEXT.md".to_string(),
                                "PLAN.md".to_string(),
                                "crates/cade-core/src/lib.rs".to_string(),
                                "crates/cade-tui/src/lib.rs".to_string(),
                                "crates/cade-gui/src/lib.rs".to_string(),
                            ];
                            let filtered: Vec<String> = files.into_iter().filter(|f| f.contains(query)).collect();
                            if !filtered.is_empty() {
                                suggestions.set(filtered);
                                show_suggestions.set(true);
                                active_index.set(0);
                            } else {
                                show_suggestions.set(false);
                            }
                        } else {
                            show_suggestions.set(false);
                        }
                    },
                    onkeydown: move |e| {
                        if show_suggestions() && !suggestions().is_empty() {
                            match e.key() {
                                Key::ArrowDown => {
                                    e.stop_propagation();
                                    active_index.set((active_index() + 1) % suggestions().len());
                                }
                                Key::ArrowUp => {
                                    e.stop_propagation();
                                    active_index.set((active_index() + suggestions().len() - 1) % suggestions().len());
                                }
                                Key::Enter => {
                                    e.stop_propagation();
                                    let s_val = suggestions()[active_index()].clone();
                                    let text = input_text();
                                    if text.starts_with('/') {
                                        input_text.set(format!("{} ", s_val));
                                    } else if let Some(pos) = text.rfind('@') {
                                        let prefix = &text[..pos];
                                        input_text.set(format!("{}@{} ", prefix, s_val));
                                    }
                                    show_suggestions.set(false);
                                }
                                Key::Escape => {
                                    e.stop_propagation();
                                    show_suggestions.set(false);
                                }
                                _ => {}
                            }
                        } else if e.key() == Key::Enter && !e.modifiers().shift() {
                            e.stop_propagation();
                            do_send();
                        }
                    }
                }
                div { class: "flex items-center justify-between pt-2 border-t border-[#1e293b]/40 select-none",
                    div { class: "flex items-center space-x-3 text-xs text-gray-500 font-medium",
                        span { class: "flex items-center space-x-1",
                            span { class: "text-emerald-500", "\u{1f7e2}" }
                            span { "Cloud" }
                        }
                        span { class: "flex items-center space-x-1",
                            span { "\u{1f4c1}" }
                            span { "root" }
                        }
                    }
                    button {
                        class: if is_loading() { "w-7 h-7 bg-sky-500 text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150 opacity-50 cursor-not-allowed" } else { "w-7 h-7 bg-sky-500 text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150" },
                        onclick: move |_| do_send(),
                        svg { class: "w-4 h-4 transform rotate-90", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2.5",
                            path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M12 19V5m-7 7l7-7 7 7" }
                        }
                    }
                }
            }
        }
    }
}

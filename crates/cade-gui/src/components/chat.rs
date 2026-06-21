use dioxus::prelude::*;

use crate::api;
use crate::types::AppState;

/// Full chat view with message history and input area.
#[component]
pub fn ChatView() -> Element {
    let state = use_context::<AppState>();
    let agent_name = (state
        .selected_agent)()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "deep-thought-research-agent_copy".to_string());

    rsx! {
        div { class: "flex flex-1 h-full overflow-hidden",
            // Sub-Sidebar
            chat_sidebar { agent_name: agent_name.clone() }

            // Main Chat Panel
            div { class: "flex-1 flex flex-col justify-between bg-[#0f1115] h-full",
                // Header bar
                header { class: "px-6 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                    span { class: "text-white font-medium text-sm", "Main chat" }
                }

            // Messages area
            messages_panel { messages: state.messages, agent_name: agent_name.clone() }

            // Input area
            input_area {
                input_text: state.input_text,
                is_loading: state.is_loading,
                messages: state.messages,
                selected_agent: state.selected_agent,
                api_key: state.api_key
            }
            }
        }
    }
}

/// Left sub-sidebar in the chat view.
#[component]
fn chat_sidebar(agent_name: String) -> Element {
    rsx! {
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
                        span { "\u{1f9e0}" }
                        span { "Memory" }
                    }
                    div { class: "flex items-center space-x-2.5 px-3 py-2 rounded-md hover:bg-[#1f212a] hover:text-white cursor-pointer transition duration-150",
                        span { "\u{1f4dd}" }
                        span { "New chat" }
                    }
                }
                // Pinned Section
                div { class: "flex flex-col space-y-1",
                    div { class: "text-[10px] font-bold text-gray-500 px-3 tracking-wider uppercase", "Pinned" }
                    div { class: "flex items-center justify-between px-3 py-2 rounded-md bg-[#1f212a]/60 text-white font-medium cursor-pointer",
                        div { class: "flex items-center space-x-2.5",
                            span { "\u{1f4ac}" }
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
    }
}

/// Scrollable message history panel.
#[component]
fn messages_panel(
    messages: Signal<Vec<cade_api_types::ChatMessage>>,
    agent_name: String,
) -> Element {
    rsx! {
        div { class: "flex-1 overflow-y-auto p-8 space-y-6 flex flex-col",
            if messages().is_empty() {
                div { class: "m-auto flex flex-col items-center select-none",
                    div { class: "w-16 h-16 rounded-xl bg-gradient-to-tr from-[#ec4899] to-[#8b5cf6] filter drop-shadow-[0_0_12px_rgba(236,72,153,0.4)] mb-4" }
                    h2 { class: "text-[24px] font-semibold text-white mb-6", "Hi, I'm {agent_name}" }
                }
            } else {
                for m in messages().iter() {
                    message_bubble { message: m.clone() }
                }
            }
        }
    }
}

/// Single chat message bubble.
#[component]
fn message_bubble(message: cade_api_types::ChatMessage) -> Element {
    let is_user = message.role == "user";
    let content_str;
    let content_val = if let Some(s) = message.content.as_str() {
        s
    } else {
        content_str = message.content.to_string();
        &content_str
    };
    let is_streaming = message.id.starts_with("streaming-");
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
                div { class: "text-[10px] font-bold text-gray-500 uppercase select-none mb-1",
                    if is_user { "user" } else if is_streaming { "assistant (streaming…)" } else { "assistant" }
                }
                p { class: "text-gray-200 mt-1 whitespace-pre-wrap",
                    "{content_val}"
                    if is_streaming && content_val.is_empty() {
                        span { class: "animate-pulse text-gray-500", "\u{25cf}" }
                    }
                }
            }
        }
    }
}

/// Text input area with send button.
#[component]
fn input_area(
    input_text: Signal<String>,
    is_loading: Signal<bool>,
    messages: Signal<Vec<cade_api_types::ChatMessage>>,
    selected_agent: Signal<Option<cade_api_types::AgentInfo>>,
    api_key: Signal<String>,
) -> Element {
    let mut do_send = move || {
        let text = input_text().trim().to_string();
        if text.is_empty() || is_loading() {
            return;
        }
        is_loading.set(true);
        input_text.set(String::new());

        let stream_id = format!("streaming-{}", js_sys::Date::now() as u64);

        // Optimistically insert user message + placeholder assistant message
        let mut current_msgs = messages();
        current_msgs.push(cade_api_types::ChatMessage {
            id: format!("user-{}", js_sys::Date::now() as u64),
            role: "user".to_string(),
            content: serde_json::Value::String(text.clone()),
            conversation_id: None,
        });
        current_msgs.push(cade_api_types::ChatMessage {
            id: stream_id.clone(),
            role: "assistant".to_string(),
            content: serde_json::Value::String(String::new()),
            conversation_id: None,
        });
        messages.set(current_msgs);

        let agent_id = selected_agent()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let key = api_key();

        spawn(async move {
            let result = api::stream_messages(&agent_id, &text, &key, |event| {
                if event.msg_type() == "assistant_message"
                    && let Some(delta) = event.content()
                {
                    let mut msgs = messages();
                    if let Some(idx) = msgs.iter().position(|m| m.id == stream_id) {
                        let existing = msgs[idx]
                            .content
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        msgs[idx].content =
                            serde_json::Value::String(format!("{existing}{delta}"));
                        messages.set(msgs);
                    }
                }
            }).await;

            // Mark streaming message as complete or show error
            let mut msgs = messages();
            let final_content = if let Err(e) = &result {
                format!("Error: {e}")
            } else {
                msgs.iter()
                    .find(|m| m.id == stream_id)
                    .and_then(|m| m.content.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            if let Some(idx) = msgs.iter().position(|m| m.id == stream_id) {
                msgs[idx].content = serde_json::Value::String(final_content);
                // Give it a stable ID now that streaming is done
                msgs[idx].id = format!("msg-{}", js_sys::Date::now() as u64);
                messages.set(msgs);
            }
            is_loading.set(false);
        });
    };

    rsx! {
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
                            do_send();
                        }
                    }
                }
                div { class: "flex items-center justify-between pt-2 border-t border-[#272833]/40 select-none",
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
                        class: if is_loading() { "w-7 h-7 bg-[#ff7c5c] text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150 opacity-50 cursor-not-allowed" } else { "w-7 h-7 bg-[#ff7c5c] text-white rounded-lg flex items-center justify-center hover:bg-[#e26a4f] transition duration-150" },
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

use crate::types::AppState;
use cade_api_types::{Question, QuestionRequest};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
    let previousFocus;
    export function openInteractionDialog() {
        const dialog = document.getElementById('cade-interaction-dialog');
        if (dialog && !dialog.open) {
            previousFocus = document.activeElement;
            dialog.showModal();
        }
    }
    export function closeInteractionDialog() {
        const dialog = document.getElementById('cade-interaction-dialog');
        if (dialog && dialog.open) dialog.close();
        if (previousFocus && previousFocus.isConnected) previousFocus.focus();
    }
")]
extern "C" {
    #[wasm_bindgen(js_name = openInteractionDialog)]
    fn open_dialog();
    #[wasm_bindgen(js_name = closeInteractionDialog)]
    fn close_dialog();
}

#[derive(Clone, Default)]
struct AnswerDraft {
    selected: Vec<Vec<String>>,
    custom: Vec<String>,
}

impl AnswerDraft {
    fn new(count: usize) -> Self {
        Self {
            selected: vec![vec![]; count],
            custom: vec![String::new(); count],
        }
    }

    fn choose(&mut self, index: usize, label: String, multiple: bool) {
        let choices = &mut self.selected[index];
        if multiple {
            if choices.contains(&label) {
                choices.retain(|v| v != &label);
            } else {
                choices.push(label);
            }
        } else {
            *choices = vec![label];
        }
    }

    /// The server expects JSON-string feedback containing header → answer.
    /// Every question must have an explicit answer; no default option is sent.
    fn feedback(&self, questions: &[Question]) -> Result<String, String> {
        let mut answers = std::collections::BTreeMap::new();
        for (index, question) in questions.iter().enumerate() {
            let custom = self.custom.get(index).map(|s| s.trim()).unwrap_or("");
            let choices = self.selected.get(index).cloned().unwrap_or_default();
            let answer = if !custom.is_empty() {
                custom.to_owned()
            } else {
                choices.join(", ")
            };
            if answer.is_empty() {
                return Err(format!("Answer ‘{}’ before continuing.", question.header));
            }
            if answers.insert(&question.header, answer).is_some() {
                return Err("Question headers must be unique to submit answers.".into());
            }
        }
        serde_json::to_string(&answers).map_err(|e| e.to_string())
    }
}

#[component]
pub fn ChatInteractions(state: AppState) -> Element {
    let agent = (state.selected_agent)().map(|a| a.id).unwrap_or_default();
    let conversation = (state.active_conversation)();
    let pending = (state.pending_approvals)().into_iter().find(|row| {
        row["agent_id"].as_str() == Some(&agent)
            && row["conversation_id"]
                .as_str()
                .is_none_or(|id| conversation.as_deref() == Some(id))
            && row["id"].as_str().is_some_and(|id| !id.is_empty())
    });
    let pending = pending.map(|row| (row["id"].as_str().unwrap_or_default().to_owned(), row));
    rsx! {
        if let Some((identity, row)) = pending {
            InteractionPrompt { key: "{identity}", state, row }
        }
    }
}

#[component]
fn InteractionPrompt(state: AppState, row: serde_json::Value) -> Element {
    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let mut expanded = use_signal(|| true);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let is_question =
        row["tool_name"].as_str() == Some("ask_user_question") || row.get("questions").is_some();
    let request = if is_question {
        QuestionRequest::from_pending(&row)
    } else {
        None
    };
    let questions = request.map(|r| r.questions).unwrap_or_default();
    let count = questions.len();
    let mut draft = use_signal(move || AnswerDraft::new(count));
    let id = row["id"].as_str().unwrap_or_default().to_owned();
    let title = if is_question {
        "Your input is needed".to_owned()
    } else {
        format!(
            "Allow {}?",
            row["tool_name"].as_str().unwrap_or("this tool")
        )
    };
    let reason = row["reason"].as_str().unwrap_or_default().to_owned();
    let arguments = cade_api_types::decode_json_value(row["arguments"].clone());
    let arguments = serde_json::to_string_pretty(&arguments).unwrap_or_default();
    let submit_id = id.clone();
    let mut submit = move |action: &'static str, feedback: Option<String>| {
        if submitting() {
            return;
        }
        submitting.set(true);
        error.set(None);
        let api = client();
        let id = submit_id.clone();
        spawn(async move {
            let response = api
                .respond_to_interaction(&id, action, feedback.as_deref())
                .await;
            if *state.api_key.peek() != api.api_key {
                return;
            }
            match response {
                Ok(_) => {
                    close_dialog();
                    crate::chat_session::ChatSessionCoordinator::pending_event(
                        state,
                        &cade_api_types::StreamEvent {
                            message_type: "approval_resolved".into(),
                            data: serde_json::json!({"id": id}),
                        },
                    );
                }
                Err(message) => {
                    submitting.set(false);
                    error.set(Some(message));
                }
            }
        });
    };
    let answer_questions = questions.clone();
    let submit_answers = submit.clone();
    let mut submit_denial = submit.clone();

    rsx! {
        div { class: "cade-prompt-bar", role: "status",
            span { "{title}" }
            button { class: "cade-prompt-button", onclick: move |_| expanded.set(true), "Review" }
        }
        if expanded() {
            dialog {
                id: "cade-interaction-dialog", class: "cade-dialog",
                "aria-labelledby": "cade-interaction-title",
                "aria-describedby": "cade-interaction-description",
                "aria-busy": submitting(),
                onmounted: move |_| open_dialog(),
                onkeydown: move |event| {
                    event.stop_propagation();
                    if event.key() == Key::Escape {
                        event.prevent_default();
                        close_dialog();
                        expanded.set(false);
                    }
                },
                header {
                    div {
                        h2 { id: "cade-interaction-title", "{title}" }
                        p { class: "prompt-muted", "{id}" }
                    }
                    button { class: "cade-prompt-button", "aria-label": "Review later", onclick: move |_| { close_dialog(); expanded.set(false); }, "Later" }
                }
                div { class: "prompt-body",
                    p { id: "cade-interaction-description", class: "prompt-muted",
                        if is_question { "Answer the agent’s questions to continue this run." }
                        else { "Review the requested tool and its arguments before allowing execution." }
                    }
                    if !reason.is_empty() { p { "{reason}" } }
                    if is_question {
                        if questions.is_empty() {
                            p { class: "prompt-error", role: "alert", "This question request could not be decoded. Its original arguments are shown below." }
                            pre { "{arguments}" }
                        }
                        for (index, question) in questions.iter().enumerate() {
                            fieldset { disabled: submitting(),
                                legend { "{question.header} — {question.question}" }
                                for option in &question.options {
                                    {
                                        let label = option.label.clone();
                                        let selected = draft.read().selected.get(index).is_some_and(|choices| choices.contains(&label));
                                        let multiple = question.multi_select;
                                        rsx! {
                                            label { class: "prompt-choice",
                                                input {
                                                    r#type: if multiple { "checkbox" } else { "radio" },
                                                    name: "question-{index}", checked: selected,
                                                    onchange: move |_| draft.write().choose(index, label.clone(), multiple),
                                                }
                                                span {
                                                    strong { "{option.label}" }
                                                    if !option.description.is_empty() { span { class: "prompt-muted", "{option.description}" } }
                                                }
                                            }
                                        }
                                    }
                                }
                                label { r#for: "question-custom-{index}", class: "prompt-muted", "Or write your own answer" }
                                textarea {
                                    id: "question-custom-{index}", value: draft.read().custom.get(index).cloned().unwrap_or_default(),
                                    oninput: move |event| draft.write().custom[index] = event.value(),
                                }
                            }
                        }
                    } else { pre { "{arguments}" } }
                    if let Some(message) = error() { p { class: "prompt-error", role: "alert", "{message}" } }
                }
                footer {
                    button { class: "cade-prompt-button", disabled: submitting(), onclick: move |_| submit_denial("deny", None),
                        if is_question { "Decline" } else { "Deny" }
                    }
                    if is_question {
                        button { class: "cade-prompt-button primary", disabled: submitting() || questions.is_empty(),
                            onclick: move |_| {
                                match draft.read().feedback(&answer_questions) {
                                    Ok(feedback) => { let mut send = submit_answers.clone(); send("approve", Some(feedback)); }
                                    Err(message) => error.set(Some(message)),
                                }
                            },
                            if submitting() { "Submitting…" } else { "Send answers" }
                        }
                    } else {
                        button { class: "cade-prompt-button primary", disabled: submitting(), onclick: move |_| submit("approve", None),
                            if submitting() { "Submitting…" } else { "Allow once" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn question_answers_require_explicit_input_and_use_server_feedback_contract() {
        let request = QuestionRequest::from_pending(&json!({"id":"q-1","arguments":
            "{\"questions\":[{\"header\":\"Target\",\"question\":\"Which platforms?\",\"multiSelect\":true,\"options\":[{\"label\":\"Web\"},{\"label\":\"CLI\"}]}]}"})).unwrap();
        let mut draft = AnswerDraft::new(1);
        assert!(draft.feedback(&request.questions).is_err());
        draft.choose(0, "Web".into(), true);
        draft.choose(0, "CLI".into(), true);
        let answers: serde_json::Value =
            serde_json::from_str(&draft.feedback(&request.questions).unwrap()).unwrap();
        assert_eq!(answers, json!({"Target":"Web, CLI"}));
        draft.custom[0] = " Desktop ".into();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&draft.feedback(&request.questions).unwrap())
                .unwrap(),
            json!({"Target":"Desktop"})
        );
    }

    #[test]
    fn question_answers_single_select_replaces_choices_and_rejects_ambiguous_headers() {
        let question: Question = serde_json::from_value(json!({"header":"Target","question":"Which target?","options":[{"label":"Web"},{"label":"CLI"}]})).unwrap();
        let mut draft = AnswerDraft::new(1);
        draft.choose(0, "Web".into(), false);
        draft.choose(0, "CLI".into(), false);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &draft.feedback(std::slice::from_ref(&question)).unwrap()
            )
            .unwrap(),
            json!({"Target":"CLI"})
        );
        let duplicate_headers = vec![question.clone(), question];
        let mut ambiguous = AnswerDraft::new(2);
        ambiguous.custom = vec!["Web".into(), "CLI".into()];
        assert!(ambiguous.feedback(&duplicate_headers).is_err());
    }
}

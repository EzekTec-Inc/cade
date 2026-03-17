/// AskUserQuestion tool — lets the LLM ask the user structured questions
/// with labelled multiple-choice options (single- or multi-select).
///
/// The tool itself never executes via `dispatch()`.  It is intercepted in
/// `execute_tool()` before the normal permission/hook pipeline, handled by
/// `Repl::handle_ask_user_question()` which drives the interactive TUI.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

// ── Data types ────────────────────────────────────────────────────────────────

/// One option inside a question.
#[derive(Debug, Clone)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

/// One question (parsed from the LLM's tool arguments).
#[derive(Debug, Clone)]
pub struct AskQuestion {
    /// Full question text shown to the user.
    pub question: String,
    /// Short chip label (≤12 chars) — used as the map key in answers.
    pub header: String,
    /// 2–4 answer options.
    pub options: Vec<AskOption>,
    /// Whether the user may select multiple options.
    pub multi_select: bool,
}

// ── AskUserQuestionTool ───────────────────────────────────────────────────────

pub struct AskUserQuestionTool;

impl AskUserQuestionTool {
    /// JSON schema definition returned to the LLM.
    pub fn schema() -> Value {
        json!({
            "name": "ask_user_question",
            "description": "Ask the user 1–4 clarifying questions with labelled multiple-choice options. \
                            Use when a decision, preference, or trade-off is needed before proceeding. \
                            Each question may be single-select or multi-select. \
                            A free-text 'Other' option is always appended automatically.",
            "parameters": {
                "type": "object",
                "required": ["questions"],
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "List of 1 to 4 questions to present to the user.",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "required": ["question", "header", "options", "multiSelect"],
                            "properties": {
                                "question": {
                                    "type": "string",
                                    "description": "The full question text shown to the user."
                                },
                                "header": {
                                    "type": "string",
                                    "description": "Short chip label shown above the question (max 12 chars)."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "2 to 4 answer choices.",
                                    "minItems": 2,
                                    "maxItems": 4,
                                    "items": {
                                        "type": "object",
                                        "required": ["label", "description"],
                                        "properties": {
                                            "label": {
                                                "type": "string",
                                                "description": "Short option label (1–5 words)."
                                            },
                                            "description": {
                                                "type": "string",
                                                "description": "One-sentence explanation of what this option means."
                                            }
                                        }
                                    }
                                },
                                "multiSelect": {
                                    "type": "boolean",
                                    "description": "If true, the user may select multiple options. \
                                                    If false (default), only one option can be selected."
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    /// Parse and validate `questions` from the tool's JSON arguments.
    ///
    /// Returns an error with a clear message if the schema is violated.
    pub fn parse_questions(args: &Value) -> Result<Vec<AskQuestion>> {
        let arr = args["questions"]
            .as_array()
            .ok_or_else(|| anyhow!("'questions' must be an array"))?;

        if arr.is_empty() || arr.len() > 4 {
            return Err(anyhow!("'questions' must contain 1 to 4 items (got {})", arr.len()));
        }

        let mut questions = Vec::with_capacity(arr.len());
        for (qi, item) in arr.iter().enumerate() {
            let question = item["question"]
                .as_str()
                .ok_or_else(|| anyhow!("questions[{qi}].question must be a string"))?
                .to_string();

            let header = item["header"]
                .as_str()
                .ok_or_else(|| anyhow!("questions[{qi}].header must be a string"))?
                .to_string();

            let multi_select = item["multiSelect"]
                .as_bool()
                .unwrap_or(false);

            let opts_arr = item["options"]
                .as_array()
                .ok_or_else(|| anyhow!("questions[{qi}].options must be an array"))?;

            if opts_arr.len() < 2 || opts_arr.len() > 4 {
                return Err(anyhow!(
                    "questions[{qi}].options must have 2–4 items (got {})",
                    opts_arr.len()
                ));
            }

            let mut options = Vec::with_capacity(opts_arr.len());
            for (oi, opt) in opts_arr.iter().enumerate() {
                let label = opt["label"]
                    .as_str()
                    .ok_or_else(|| anyhow!("questions[{qi}].options[{oi}].label must be a string"))?
                    .to_string();
                let description = opt["description"]
                    .as_str()
                    .ok_or_else(|| anyhow!("questions[{qi}].options[{oi}].description must be a string"))?
                    .to_string();
                options.push(AskOption { label, description });
            }

            questions.push(AskQuestion { question, header, options, multi_select });
        }

        Ok(questions)
    }

    /// Format the tool result string returned to the LLM after answers are collected.
    ///
    /// Output (CADE Code parity):
    /// `User has answered your questions: "Q1"="A1", "Q2"="A2". You can now continue.`
    pub fn format_result(answers: &HashMap<String, String>) -> String {
        if answers.is_empty() {
            return "User provided no answers.".to_string();
        }
        let pairs: Vec<String> = answers
            .iter()
            .map(|(q, a)| format!("\"{q}\"=\"{a}\""))
            .collect();
        format!(
            "User has answered your questions: {}. You can now continue with the user's answers in mind.",
            pairs.join(", ")
        )
    }
}

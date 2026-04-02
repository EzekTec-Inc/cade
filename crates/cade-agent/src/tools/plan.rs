use crate::Result;
use serde_json::{Value, json};

pub struct EnterPlanModeTool;
impl EnterPlanModeTool {
    pub fn schema() -> Value {
        json!({
            "name": "EnterPlanMode",
            "description": "Enter a read-only planning mode. Use this when you need to explore the codebase or gather information without making any permanent changes.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        })
    }
}

pub struct ExitPlanModeTool;
impl ExitPlanModeTool {
    pub fn schema() -> Value {
        json!({
            "name": "ExitPlanMode",
            "description": "Exit the read-only planning mode and resume normal operation.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        })
    }
}

// -- Scratchpad / memory persistence

/// Writes the agent's task list to `.cade-todo.md` in the current directory.
/// Use this to persist a scratchpad across conversation turns.
pub struct TodoWriteTool;
impl TodoWriteTool {
    pub fn schema() -> Value {
        json!({
            "name": "TodoWrite",
            "description": "Write your current plan or scratchpad to a todo file. Use this to keep track of tasks across steps.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The plan or todo list content"
                    }
                },
                "required": ["content"]
            }
        })
    }

    pub async fn run(args: &Value) -> Result<String> {
        let content = args["content"].as_str().unwrap_or("");
        let path = std::env::current_dir()?.join(".cade-todo.md");
        std::fs::write(&path, content)?;
        Ok(format!("Successfully updated {}", path.display()))
    }
}

// -- Live plan panel (TUI overlay)

/// Sets the numbered steps shown in the TUI plan panel.
/// Each step is displayed as a checklist the user can see in real time.
/// Call this once at the start of a multi-step task to establish the plan.
pub struct SetPlanTool;
impl SetPlanTool {
    pub fn schema() -> Value {
        json!({
            "name": "set_plan",
            "description": "Set the numbered steps shown in the TUI plan panel. Call once at the start of a multi-step task. Each step appears as a checklist item the user can track in real time.",
            "parameters": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "Ordered list of step descriptions.",
                        "items": { "type": "string" }
                    }
                },
                "required": ["steps"]
            }
        })
    }
}

/// Marks a step in the TUI plan panel as done (or not done).
/// step_id is 1-based, matching the position in the steps array passed to set_plan.
pub struct UpdatePlanTool;
impl UpdatePlanTool {
    pub fn schema() -> Value {
        json!({
            "name": "UpdatePlan",
            "description": "Mark a step in the TUI plan panel as done or not done. step_id is 1-based.",
            "parameters": {
                "type": "object",
                "properties": {
                    "step_id": {
                        "type": "integer",
                        "description": "1-based index of the step to update."
                    },
                    "done": {
                        "type": "boolean",
                        "description": "true to mark the step complete, false to unmark it."
                    }
                },
                "required": ["step_id", "done"]
            }
        })
    }
}

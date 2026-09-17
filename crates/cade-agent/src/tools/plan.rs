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
            "description": "Set the numbered steps shown in the plan panel. Call once at the start of a multi-step task. Each step appears as a checklist item the user can track in real time. CRITICAL: You MUST use the UpdatePlan tool to mark steps as done immediately as you finish them.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Optional title for the plan/checklist (e.g. 'Database Migration'). Defaults to 'Tasks'."
                    },
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

    pub async fn run(args: &Value) -> Result<String> {
        let steps = args
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| crate::Error::custom("Missing required 'steps' array in set_plan"))?;
        let count = steps.len();
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Tasks");
        Ok(format!("Plan '{title}' initialized with {count} steps."))
    }
}

/// Marks a step in the TUI plan panel as done (or not done).
/// step_id is 1-based, matching the position in the steps array passed to set_plan.
pub struct UpdatePlanTool;
impl UpdatePlanTool {
    pub fn schema() -> Value {
        json!({
            "name": "UpdatePlan",
            "description": "Mark a step in the TUI plan panel as done or not done. step_id is 1-based. CRITICAL: You MUST call this tool immediately upon finishing the work for a step. Never conclude a response with unfinished steps if the work is actually complete.",
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

    pub async fn run(args: &Value) -> Result<String> {
        let step_id = args
            .get("step_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| crate::Error::custom("Missing required 'step_id' in UpdatePlan"))?;
        let done = args
            .get("done")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| crate::Error::custom("Missing required 'done' in UpdatePlan"))?;
        let status = if done { "completed" } else { "pending" };
        Ok(format!("Step {step_id} marked as {status}."))
    }
}
/// Finishes the current task, generating an automated audit changelog and optionally committing.
pub struct FinishTaskTool;
impl FinishTaskTool {
    pub fn schema() -> Value {
        json!({
            "name": "finish_task",
            "description": "Call this tool when you have completed a task. The server will automatically generate an audit log, rollback steps, and optionally commit the changes. Replaces the manual PLAN.md process.",
            "parameters": {
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "A brief summary of what was accomplished."
                    },
                    "reason": {
                        "type": "string",
                        "description": "The reason for this change."
                    }
                },
                "required": ["summary", "reason"]
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_plan_tool_run_success() {
        let args = json!({
            "title": "Refactor Viewport",
            "steps": ["Step 1: Explore", "Step 2: Implement", "Step 3: Test"]
        });
        let result = SetPlanTool::run(&args).await.unwrap();
        assert!(result.contains("Refactor Viewport"));
        assert!(result.contains("3 steps"));
    }

    #[tokio::test]
    async fn test_set_plan_tool_missing_steps() {
        let args = json!({ "title": "Missing Steps" });
        let err = SetPlanTool::run(&args).await.unwrap_err();
        assert!(err.to_string().contains("Missing required 'steps'"));
    }

    #[tokio::test]
    async fn test_update_plan_tool_run_done() {
        let args = json!({ "step_id": 2, "done": true });
        let result = UpdatePlanTool::run(&args).await.unwrap();
        assert!(result.contains("Step 2 marked as completed"));
    }

    #[tokio::test]
    async fn test_update_plan_tool_run_pending() {
        let args = json!({ "step_id": 1, "done": false });
        let result = UpdatePlanTool::run(&args).await.unwrap();
        assert!(result.contains("Step 1 marked as pending"));
    }
}

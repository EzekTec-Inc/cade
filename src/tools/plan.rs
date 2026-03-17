use anyhow::Result;
use serde_json::Value;

pub struct EnterPlanModeTool;
impl EnterPlanModeTool {
    pub fn schema() -> Value {
        serde_json::json!({
            "name": "EnterPlanMode",
            "description": "Enter a read-only planning mode. Use this when you need to explore the codebase or gather information without making any permanent changes.",
            "input_schema": {
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
        serde_json::json!({
            "name": "ExitPlanMode",
            "description": "Exit the read-only planning mode and resume normal operation.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        })
    }
}

pub struct TodoWriteTool;
impl TodoWriteTool {
    pub fn schema() -> Value {
        serde_json::json!({
            "name": "TodoWrite",
            "description": "Write your current plan or scratchpad to a todo file. Use this to keep track of tasks across steps.",
            "input_schema": {
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

pub struct UpdatePlanTool;
impl UpdatePlanTool {
    pub fn schema() -> Value {
        serde_json::json!({
            "name": "UpdatePlan",
            "description": "Update your plan or scratchpad.",
            "input_schema": {
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
}

pub struct WriteTodosTool;
impl WriteTodosTool {
    pub fn schema() -> Value {
        serde_json::json!({
            "name": "WriteTodos",
            "description": "Write your current plan or scratchpad to a todo file.",
            "input_schema": {
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
}

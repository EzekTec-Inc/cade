
use crate::moa::{Agent, AgentRequest, AgentResponse, AgentResult};
use crate::tools::fs::{ReadTool, WriteTool, EditTool, ApplyPatchTool};
use async_trait::async_trait;
use serde_json::{json, Value};

fn parse_json_args(prompt: &str) -> Option<Value> {
    if let Some(start_idx) = prompt.find('{')
        && let Some(end_idx) = prompt.rfind('}')
        && start_idx < end_idx
    {
        let json_str = &prompt[start_idx..=end_idx];
        return serde_json::from_str(json_str).ok();
    }
    None
}

pub struct ReadToolAgent;

#[async_trait]
impl Agent for ReadToolAgent {
    fn name(&self) -> &str {
        "read_file_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["read file".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["read_file"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        let args = if let Some(json) = parse_json_args(&request.prompt) {
            json
        } else {
            let path = request.prompt.split_whitespace().last().unwrap_or("");
            json!({ "path": path })
        };

        match ReadTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("read_file_agent failed: {}", e))),
        }
    }
}

pub struct WriteToolAgent;

#[async_trait]
impl Agent for WriteToolAgent {
    fn name(&self) -> &str {
        "write_file_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["write file".to_string(), "create file".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["write_file"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        let args = if let Some(json) = parse_json_args(&request.prompt) {
            json
        } else {
            let parts: Vec<&str> = request.prompt.split_whitespace().collect();
            if parts.len() < 4 {
                return Err(Box::from("Invalid write file command: requires path and content"));
            }
            let path = parts[2];
            let content = parts[3..].join(" ");
            json!({ "path": path, "content": content })
        };

        match WriteTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("write_file_agent failed: {}", e))),
        }
    }
}

pub struct EditToolAgent;

#[async_trait]
impl Agent for EditToolAgent {
    fn name(&self) -> &str {
        "edit_file_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["edit file".to_string(), "replace string".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["edit_file"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        let args = if let Some(json) = parse_json_args(&request.prompt) {
            json
        } else {
            let mut path = "";
            let mut old_string = "";
            let mut new_string = "";

            if let Some(p) = request.prompt.split_whitespace().nth(2) {
                path = p;
            }

            if let Some(captures) = regex::Regex::new(r"'(.*?)' '(.*?)'").unwrap().captures(&request.prompt) {
                if let Some(old) = captures.get(1) {
                    old_string = old.as_str();
                }
                if let Some(new) = captures.get(2) {
                    new_string = new.as_str();
                }
            }
            
            if path.is_empty() || old_string.is_empty() {
                return Err(Box::from("Invalid edit file command: requires path, 'old string', and 'new string'"));
            }

            json!({ "path": path, "old_string": old_string, "new_string": new_string })
        };

        match EditTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("edit_file_agent failed: {}", e))),
        }
    }
}

pub struct ApplyPatchToolAgent;

#[async_trait]
impl Agent for ApplyPatchToolAgent {
    fn name(&self) -> &str {
        "apply_patch_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["apply patch".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["apply_patch"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        let args = if let Some(json) = parse_json_args(&request.prompt) {
            json
        } else {
            json!({ "patch": request.prompt })
        };

        match ApplyPatchTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("apply_patch_agent failed: {}", e))),
        }
    }
}

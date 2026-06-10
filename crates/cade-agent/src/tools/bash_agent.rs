use crate::moa::{Agent, AgentRequest, AgentResponse, AgentResult};
use crate::tools::bash::BashTool;
use async_trait::async_trait;
use serde_json::json;

pub struct BashToolAgent;

#[async_trait]
impl Agent for BashToolAgent {
    fn name(&self) -> &str {
        "bash_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["bash".to_string(), "shell".to_string(), "command".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["bash", "shell"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        // Extracts the command from a prompt like "bash ls -l"
        let command = request.prompt.split_once(' ').map(|(_, cmd)| cmd).unwrap_or("");
        let args = json!({ "command": command });

        match BashTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("bash_agent failed: {}", e))),
        }
    }
}

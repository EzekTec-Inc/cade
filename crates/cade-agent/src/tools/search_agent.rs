use crate::moa::{Agent, AgentRequest, AgentResponse, AgentResult};
use crate::tools::search::GrepTool;
use async_trait::async_trait;
use serde_json::json;

pub struct SearchToolAgent;

#[async_trait]
impl Agent for SearchToolAgent {
    fn name(&self) -> &str {
        "search_agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["search".to_string(), "find in files".to_string(), "grep".to_string()]
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        vec!["search", "grep"]
    }

    async fn execute(&self, request: &AgentRequest) -> AgentResult {
        // Extracts pattern from "search 'pattern'"
        let pattern = request.prompt.split_once(' ').map(|(_, p)| p).unwrap_or("");
        let args = json!({ "pattern": pattern });

        match GrepTool::run(&args).await {
            Ok(content) => Ok(AgentResponse { content }),
            Err(e) => Err(Box::from(format!("search_agent failed: {}", e))),
        }
    }
}

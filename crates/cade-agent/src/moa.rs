use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub content: String,
}

pub type AgentResult = Result<AgentResponse, Box<dyn std::error::Error + Send + Sync>>;

#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> Vec<String>;
    fn supported_tools(&self) -> Vec<&'static str>;
    async fn execute(&self, request: &AgentRequest) -> AgentResult;
}

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Input to an [`Agent::execute`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// The prompt text to send to the agent.
    pub prompt: String,
}

/// Output from an [`Agent::execute`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// The agent's reply text.
    pub content: String,
}

/// Convenience alias for fallible agent execution.
pub type AgentResult = Result<AgentResponse, Box<dyn std::error::Error + Send + Sync>>;

/// Pluggable agent abstraction (MoA — "Mixture of Agents").
///
/// Each implementation provides a `name`, `capabilities`, and a set of
/// `supported_tools`. The [`execute`](Agent::execute) method runs one
/// turn and returns the result.
#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> Vec<String>;
    fn supported_tools(&self) -> Vec<&'static str>;
    async fn execute(&self, request: &AgentRequest) -> AgentResult;
}


use crate::moa::{Agent, AgentRequest, AgentResult};
use std::sync::Arc;

#[derive(Default)]
pub struct Router {
    agents: Vec<Arc<dyn Agent>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_agent(&mut self, agent: Arc<dyn Agent>) {
        self.agents.push(agent);
    }

    pub fn route(&self, request: &AgentRequest) -> Result<Vec<Arc<dyn Agent>>, Box<dyn std::error::Error + Send + Sync>> {
        let mut selected_agents: Vec<Arc<dyn Agent>> = Vec::new();

        for agent in &self.agents {
            let mut score = 0;
            for capability in agent.capabilities() {
                if request.prompt.contains(&capability) {
                    score += 1;
                }
            }
            if score > 0 {
                selected_agents.push(agent.clone());
            }
        }

        if !selected_agents.is_empty() {
            return Ok(selected_agents);
        }

        // Default to the first agent if no capability matches
        if let Some(agent) = self.agents.first() {
            return Ok(vec![agent.clone()]);
        }

        Err(Box::from("No agents available to handle the request"))
    }
}

pub struct Aggregator;

impl Aggregator {
    pub fn aggregate(&self, results: Vec<AgentResult>) -> String {
        let mut aggregated_content = String::new();

        for result in results {
            match result {
                Ok(response) => {
                    if !aggregated_content.is_empty() {
                        aggregated_content.push_str("\n\n---\n\n");
                    }
                    aggregated_content.push_str(&response.content);
                }
                Err(e) => {
                    if !aggregated_content.is_empty() {
                        aggregated_content.push_str("\n\n---\n\n");
                    }
                    aggregated_content.push_str(&format!("Error: {}", e));
                }
            }
        }
        aggregated_content
    }
}

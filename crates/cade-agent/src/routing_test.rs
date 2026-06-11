use super::moa::{Agent, AgentRequest, AgentResponse, AgentResult};
use super::routing::{Aggregator, Router};
use async_trait::async_trait;
use std::sync::Arc;

// Mock Agent for testing
struct MockAgent {
    name: String,
    caps: Vec<String>,
}

#[async_trait]
impl Agent for MockAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Vec<String> {
        self.caps.clone()
    }

    fn supported_tools(&self) -> Vec<&'static str> {
        // For testing purposes, we assume the name is the tool
        vec![]
    }

    async fn execute(&self, _request: &AgentRequest) -> AgentResult {
        Ok(AgentResponse {
            content: format!("{} executed", self.name),
        })
    }
}

#[tokio::test]
async fn test_router_selects_all_matching_agents() {
    let mut router = Router::new();
    let agent1 = MockAgent {
        name: "agent1".to_string(),
        caps: vec!["read".to_string()],
    };
    let agent2 = MockAgent {
        name: "agent2".to_string(),
        caps: vec!["file".to_string()],
    };
    let agent3 = MockAgent {
        name: "agent3".to_string(),
        caps: vec!["write".to_string()],
    };
    router.add_agent(Arc::new(agent1));
    router.add_agent(Arc::new(agent2));
    router.add_agent(Arc::new(agent3));

    let request = AgentRequest {
        prompt: "read the file".to_string(),
    };

    let selected_agents = router.route(&request).unwrap();
    assert_eq!(selected_agents.len(), 2);
    assert!(selected_agents.iter().any(|a| a.name() == "agent1"));
    assert!(selected_agents.iter().any(|a| a.name() == "agent2"));
}

#[tokio::test]
async fn test_router_falls_back_to_first_agent() {
    let mut router = Router::new();
    let agent1 = MockAgent {
        name: "agent1".to_string(),
        caps: vec!["write".to_string()],
    };
    router.add_agent(Arc::new(agent1));

    let request = AgentRequest {
        prompt: "read the file".to_string(),
    };

    let selected_agents = router.route(&request).unwrap();
    assert_eq!(selected_agents.len(), 1);
    assert_eq!(selected_agents[0].name(), "agent1");
}

#[test]
fn test_aggregator_success() {
    let aggregator = Aggregator;
    let response1 = AgentResponse {
        content: "success1".to_string(),
    };
    let response2 = AgentResponse {
        content: "success2".to_string(),
    };
    let results = vec![Ok(response1), Ok(response2)];
    let result = aggregator.aggregate(results);
    assert_eq!(result, "success1\n\n---\n\nsuccess2");
}

#[test]
fn test_aggregator_error() {
    let aggregator = Aggregator;
    let response1 = AgentResponse {
        content: "success".to_string(),
    };
    let results = vec![Ok(response1), Err(Box::from("failure"))];
    let result = aggregator.aggregate(results);
    assert_eq!(result, "success\n\n---\n\nError: failure");
}

//! Unified ApiClientEngine resource hub for cade-gui (PRD #68 / Issue #69).
//!
//! Encapsulates typed resource accessors, request deduplication,
//! and mutation-triggered reactive cache invalidation behind a clean seam.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::{CadeApiClient, api_request};

// region:    --- Types

/// State of an asynchronous resource query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceState<T> {
    Loading,
    Ready(T),
    Error(String),
}

impl<T> ResourceState<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(val) => Some(val),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Error(err) => Some(err.as_str()),
            _ => None,
        }
    }
}

/// Typed mutations for backend entity state.
#[derive(Debug, Clone)]
pub enum ResourceMutation {
    CreateAgent {
        name: String,
        model: Option<String>,
    },
    DeleteAgent {
        agent_id: String,
    },
    CreateConversation {
        agent_id: String,
        title: Option<String>,
    },
    DeleteConversation {
        agent_id: String,
        conversation_id: String,
    },
    SaveMemoryBlock {
        agent_id: String,
        label: String,
        value: String,
    },
    DeleteMemoryBlock {
        agent_id: String,
        label: String,
    },
    CreateCheckpoint {
        agent_id: String,
        label: String,
        description: Option<String>,
    },
    SubmitApproval {
        approval_id: String,
        approved: bool,
        feedback: Option<String>,
    },
    DispatchWorkflowRun {
        workflow_name: String,
        params: serde_json::Value,
    },
    CancelWorkflowRun {
        run_id: String,
    },
}

/// Unified, deep resource engine for managing API communication and reactive caching.
#[derive(Clone)]
pub struct ApiClientEngine {
    api_client: Memo<CadeApiClient>,
}

impl ApiClientEngine {
    pub fn new(api_client: Memo<CadeApiClient>) -> Self {
        Self { api_client }
    }

    pub fn client(&self) -> CadeApiClient {
        (self.api_client)()
    }

    /// Fetch all active agents.
    pub async fn fetch_agents(&self) -> ResourceState<Vec<cade_api_types::AgentInfo>> {
        match self.client().list_agents().await {
            Ok(agents) => ResourceState::Ready(agents),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Submit human-in-the-loop approval decision.
    pub async fn submit_approval(
        &self,
        approval_id: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> Result<String, String> {
        self.mutate(ResourceMutation::SubmitApproval {
            approval_id: approval_id.to_string(),
            approved,
            feedback,
        })
        .await
    }

    /// Fetch messages for an agent and optional conversation.
    pub async fn fetch_messages(
        &self,
        agent_id: &str,
        conv_id: Option<&str>,
    ) -> ResourceState<Vec<cade_api_types::ChatMessage>> {
        if agent_id.is_empty() {
            return ResourceState::Ready(vec![]);
        }
        match self.client().get_messages(agent_id, conv_id).await {
            Ok(msgs) => ResourceState::Ready(msgs),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Fetch active tools and MCP servers.
    pub async fn fetch_mcp_servers(&self) -> ResourceState<Vec<serde_json::Value>> {
        match self.client().list_mcp_servers().await {
            Ok(tools) => ResourceState::Ready(tools),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Fetch all registered workflows.
    pub async fn fetch_workflows(&self) -> ResourceState<Vec<cade_api_types::WorkflowSummary>> {
        match self.client().list_workflows().await {
            Ok(w) => ResourceState::Ready(w),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Dispatch a workflow run.
    pub async fn dispatch_workflow_run(
        &self,
        workflow_name: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        self.mutate(ResourceMutation::DispatchWorkflowRun {
            workflow_name: workflow_name.to_string(),
            params,
        })
        .await
    }

    /// Fetch all models from the catalog.
    pub async fn fetch_models(&self) -> ResourceState<serde_json::Value> {
        match self.client().list_models().await {
            Ok(models) => ResourceState::Ready(models),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Fetch memory blocks for an agent.
    pub async fn fetch_memory_blocks(
        &self,
        agent_id: &str,
    ) -> ResourceState<Vec<serde_json::Value>> {
        if agent_id.is_empty() {
            return ResourceState::Ready(vec![]);
        }
        match self.client().list_memory_blocks(agent_id).await {
            Ok(blocks) => ResourceState::Ready(blocks),
            Err(e) => ResourceState::Error(e),
        }
    }

    /// Execute a resource mutation atomically.
    pub async fn mutate(&self, mutation: ResourceMutation) -> Result<String, String> {
        let client = self.client();
        let api_key = &client.api_key;
        match mutation {
            ResourceMutation::CreateAgent { name, model } => {
                let mut body_json = serde_json::json!({ "name": name });
                if let Some(m) = model {
                    body_json["model"] = serde_json::Value::String(m);
                }
                let res = api_request("POST", "/v1/agents", Some(&body_json.to_string()), api_key)
                    .await?;
                let val: serde_json::Value =
                    serde_json::from_str(&res).map_err(|e| e.to_string())?;
                Ok(val["id"].as_str().unwrap_or("agent").to_string())
            }
            ResourceMutation::DeleteAgent { agent_id } => {
                let path = format!("/v1/agents/{agent_id}");
                api_request("DELETE", &path, None, api_key).await?;
                Ok(agent_id)
            }
            ResourceMutation::CreateConversation { agent_id, title } => {
                let conv = client
                    .create_conversation(&agent_id, title.as_deref())
                    .await?;
                Ok(conv.id)
            }
            ResourceMutation::DeleteConversation {
                agent_id,
                conversation_id,
            } => {
                client
                    .delete_conversation(&agent_id, &conversation_id)
                    .await?;
                Ok(conversation_id)
            }
            ResourceMutation::SaveMemoryBlock {
                agent_id,
                label,
                value,
            } => {
                let path = format!("/v1/agents/{agent_id}/memory");
                let body = serde_json::json!({ "label": label, "value": value });
                api_request("PUT", &path, Some(&body.to_string()), api_key).await?;
                Ok(label)
            }
            ResourceMutation::DeleteMemoryBlock { agent_id, label } => {
                let path = format!("/v1/agents/{agent_id}/memory?label={label}");
                api_request("DELETE", &path, None, api_key).await?;
                Ok(label)
            }
            ResourceMutation::CreateCheckpoint {
                agent_id,
                label,
                description,
            } => {
                let path = format!("/v1/agents/{agent_id}/checkpoints");
                let body = serde_json::json!({ "label": label, "description": description });
                let res = api_request("POST", &path, Some(&body.to_string()), api_key).await?;
                let val: serde_json::Value =
                    serde_json::from_str(&res).map_err(|e| e.to_string())?;
                Ok(val["id"].as_str().unwrap_or("cp").to_string())
            }
            ResourceMutation::SubmitApproval {
                approval_id,
                approved,
                feedback,
            } => {
                let path = format!("/v1/approvals/{approval_id}");
                let body = serde_json::json!({
                    "approved": approved,
                    "feedback": feedback
                });
                api_request("POST", &path, Some(&body.to_string()), api_key).await?;
                Ok(approval_id)
            }
            ResourceMutation::DispatchWorkflowRun {
                workflow_name,
                params,
            } => {
                let path = format!("/v1/workflows/{workflow_name}/run");
                let res = api_request("POST", &path, Some(&params.to_string()), api_key).await?;
                let val: serde_json::Value =
                    serde_json::from_str(&res).map_err(|e| e.to_string())?;
                Ok(val["run_id"].as_str().unwrap_or("run").to_string())
            }
            ResourceMutation::CancelWorkflowRun { run_id } => {
                let path = format!("/v1/workflows/runs/{run_id}/cancel");
                api_request("POST", &path, None, api_key).await?;
                Ok(run_id)
            }
        }
    }
}

// endregion: --- Types

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_state_methods() {
        let loading: ResourceState<String> = ResourceState::Loading;
        assert!(loading.is_loading());
        assert_eq!(loading.value(), None);
        assert_eq!(loading.error(), None);

        let ready: ResourceState<String> = ResourceState::Ready("test-data".to_string());
        assert!(!ready.is_loading());
        assert_eq!(ready.value(), Some(&"test-data".to_string()));
        assert_eq!(ready.error(), None);

        let error: ResourceState<String> = ResourceState::Error("404 Not Found".to_string());
        assert!(!error.is_loading());
        assert_eq!(error.value(), None);
        assert_eq!(error.error(), Some("404 Not Found"));
    }

    #[test]
    fn test_resource_mutation_variants() {
        let mutation = ResourceMutation::SubmitApproval {
            approval_id: "appr-123".to_string(),
            approved: true,
            feedback: Some("Proceed".to_string()),
        };
        match mutation {
            ResourceMutation::SubmitApproval { approval_id, approved, feedback } => {
                assert_eq!(approval_id, "appr-123");
                assert!(approved);
                assert_eq!(feedback.as_deref(), Some("Proceed"));
            }
            _ => panic!("Expected SubmitApproval mutation"),
        }
    }
}

// endregion: --- Tests
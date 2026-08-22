use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Scope choice selected by user during interactive consent negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsentChoice {
    AllowOnce,
    AllowSession,
    AlwaysAllow,
    Deny,
}

#[async_trait]
pub trait PermissionService: Send + Sync {
    /// Request permission to execute a tool. Returns true if approved, false if denied.
    async fn request_permission(&self, tool_name: &str, args: &Value) -> Result<bool, String>;

    /// Request rich consent with scope choices (AllowOnce, AllowSession, AlwaysAllow, Deny).
    async fn request_consent(
        &self,
        tool_name: &str,
        args: &Value,
    ) -> Result<ConsentChoice, String> {
        match self.request_permission(tool_name, args).await {
            Ok(true) => Ok(ConsentChoice::AllowOnce),
            Ok(false) => Ok(ConsentChoice::Deny),
            Err(e) => Err(e),
        }
    }
}

pub struct YoloBypassAdapter;

#[async_trait]
impl PermissionService for YoloBypassAdapter {
    async fn request_permission(&self, _tool_name: &str, _args: &Value) -> Result<bool, String> {
        Ok(true)
    }
}

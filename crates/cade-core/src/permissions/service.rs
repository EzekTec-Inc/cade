use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait PermissionService: Send + Sync {
    /// Request permission to execute a tool. Returns true if approved, false if denied.
    async fn request_permission(&self, tool_name: &str, args: &Value) -> Result<bool, String>;
}

pub struct YoloBypassAdapter;

#[async_trait]
impl PermissionService for YoloBypassAdapter {
    async fn request_permission(&self, _tool_name: &str, _args: &Value) -> Result<bool, String> {
        Ok(true)
    }
}

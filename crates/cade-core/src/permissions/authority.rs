use crate::permissions::manager::PermissionManager;
use crate::permissions::service::PermissionService;
use crate::permissions::rules::Verdict;
use std::sync::Arc;

/// A deep, unified module that acts as the single entrypoint for security and execution permissions.
/// It brings together the synchronous rules engine (`PermissionManager`) and the asynchronous 
/// human-in-the-loop prompt delegates (`PermissionService`) behind a single, high-leverage seam.
pub struct SecurityAuthority {
    manager: PermissionManager,
    service: Arc<dyn PermissionService>,
}

impl SecurityAuthority {
    pub fn new(manager: PermissionManager, service: Arc<dyn PermissionService>) -> Self {
        Self { manager, service }
    }

    /// Evaluates permissions for a tool call. If the rules engine allows or denies the execution,
    /// it returns immediately. If the rules require explicit user confirmation (Verdict::Ask),
    /// it dynamically suspends and delegates execution to the registered permission service.
    pub async fn authorize(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        is_mcp_write: bool,
    ) -> Result<Verdict, String> {
        match self.manager.resolve(tool_name, args, is_mcp_write) {
            Verdict::Allow => Ok(Verdict::Allow),
            Verdict::Deny(reason) => Ok(Verdict::Deny(reason)),
            Verdict::Ask(reason) => {
                // Delegate to asynchronous human-in-the-loop PermissionService
                match self.service.request_permission(tool_name, args).await {
                    Ok(true) => Ok(Verdict::Allow),
                    Ok(false) => Ok(Verdict::Deny(format!("denied by user: {reason}"))),
                    Err(err) => {
                        // Pass along specific denial error feedback or general errors
                        if err.starts_with("Permission Denied:") {
                            Ok(Verdict::Deny(err))
                        } else {
                            Err(err)
                        }
                    }
                }
            }
        }
    }
}

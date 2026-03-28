/// Re-export the standalone `cade-mcp` crate so existing `crate::mcp::*` paths
/// throughout `cade-agent` and downstream crates resolve unchanged.

#[cfg(feature = "mcp")]
pub use cade_mcp::*;

/// Stub McpManager when MCP feature is disabled.
#[cfg(not(feature = "mcp"))]
mod stub {
    use serde_json::Value;

    pub struct McpManager;

    impl McpManager {
        pub fn empty() -> Self {
            McpManager
        }
        pub async fn is_empty(&self) -> bool {
            true
        }
        pub async fn call_tool(
            &self,
            _name: &str,
            _args: &Value,
        ) -> Option<Result<(String, bool), cade_core::Error>> {
            None
        }
        pub async fn owns_tool(&self, _name: &str) -> bool {
            false
        }
        pub async fn is_write_tool(&self, _name: &str) -> bool {
            false
        }
        pub async fn all_tool_schemas(&self) -> Vec<Value> {
            vec![]
        }
        pub async fn status(&self) -> Vec<Value> {
            vec![]
        }
    }
}

#[cfg(not(feature = "mcp"))]
pub use stub::*;

#[cfg(feature = "mcp")]
pub mod watcher {
    pub use cade_mcp::watcher::*;
}

#[cfg(not(feature = "mcp"))]
pub mod watcher {
    pub fn spawn_mcp_watcher(_cwd: &std::path::Path) -> tokio::sync::mpsc::Receiver<()> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        rx
    }
}

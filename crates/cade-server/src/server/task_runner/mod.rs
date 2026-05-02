//! Background task execution engine for running subagents natively on the server.

use std::sync::Arc;

use cade_agent::agent::HttpTransport;
use cade_agent::mcp::McpManager;
use cade_core::hooks::HookEngine;
use cade_core::permissions::PermissionManager;

pub mod loop_impl;

pub struct TaskRunner {
    pub client: HttpTransport,
    pub mcp: Arc<McpManager>,
    pub permissions: PermissionManager,
    pub hooks: HookEngine,
}

impl TaskRunner {
    pub fn new(
        client: HttpTransport,
        mcp: Arc<McpManager>,
        permissions: PermissionManager,
        hooks: HookEngine,
    ) -> Self {
        Self {
            client,
            mcp,
            permissions,
            hooks,
        }
    }
}

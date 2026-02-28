use std::sync::{Arc, Mutex};

/// Permission mode controlling how tool calls are approved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Prompt user for each tool call (default)
    Default,
    /// Auto-allow Write/Edit file operations
    AcceptEdits,
    /// Read-only — block all write/execute operations
    Plan,
    /// Allow all tools without prompting (--yolo)
    BypassPermissions,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Default
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::AcceptEdits => write!(f, "acceptEdits"),
            Self::Plan => write!(f, "plan"),
            Self::BypassPermissions => write!(f, "bypassPermissions"),
        }
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "default" => Ok(Self::Default),
            "acceptEdits" => Ok(Self::AcceptEdits),
            "plan" => Ok(Self::Plan),
            "bypassPermissions" => Ok(Self::BypassPermissions),
            other => anyhow::bail!("unknown permission mode: {other}"),
        }
    }
}

#[derive(Clone, Default)]
pub struct PermissionManager {
    mode: Arc<Mutex<PermissionMode>>,
}

impl PermissionManager {
    pub fn new(mode: PermissionMode) -> Self {
        Self { mode: Arc::new(Mutex::new(mode)) }
    }

    pub fn mode(&self) -> PermissionMode {
        *self.mode.lock().unwrap()
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        *self.mode.lock().unwrap() = mode;
    }

    /// Returns true if the tool call should proceed without prompting
    pub fn auto_approve(&self, tool_name: &str) -> bool {
        match self.mode() {
            PermissionMode::BypassPermissions => true,
            PermissionMode::AcceptEdits => {
                matches!(tool_name, "write_file" | "edit_file")
            }
            PermissionMode::Plan => false,
            PermissionMode::Default => false,
        }
    }

    /// Returns true if the tool call is blocked by this mode
    pub fn is_blocked(&self, tool_name: &str) -> bool {
        if self.mode() == PermissionMode::Plan {
            matches!(tool_name, "bash" | "write_file" | "edit_file")
        } else {
            false
        }
    }
}

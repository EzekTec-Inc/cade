//! Capability gating helpers for REPL commands.
//!
//! When a command requires a capability that is not enabled, the helper
//! prints a user-friendly hint and returns `true` (= blocked).

use super::Repl;
use cade_core::capabilities::{Capability, CapabilitySet};

impl Repl {
    /// Check if a capability is enabled. If not, print a hint and return `true` (blocked).
    pub(crate) fn require_capability(&self, cap: Capability, command: &str) -> bool {
        if self.capabilities.is_enabled(cap) {
            return false; // not blocked
        }
        self.tui_dim(format!(
            "  {command} requires the '{}' capability.",
            cap.name()
        ));
        self.tui_dim(format!(
            "  Enable it: add \"enable_capabilities\": [\"{}\"] to ~/.cade/settings.json",
            cap.name()
        ));
        self.tui_dim("  Or switch profile: set \"profile\": \"full\" in settings.");
        true // blocked
    }

    /// Check if any of the listed capabilities is enabled.
    #[allow(dead_code)]
    pub(crate) fn require_any_capability(&self, caps: &[Capability], command: &str) -> bool {
        if caps.iter().any(|c| self.capabilities.is_enabled(*c)) {
            return false;
        }
        let names: Vec<&str> = caps.iter().map(|c| c.name()).collect();
        self.tui_dim(format!("  {command} requires one of: {}", names.join(", ")));
        self.tui_dim("  Enable in ~/.cade/settings.json or set \"profile\": \"full\".");
        true
    }
}

/// Returns a filtered list of slash-command help entries based on capabilities.
/// Each entry is (command, description, required_capability).
#[allow(dead_code)]
pub(crate) fn visible_commands(caps: &CapabilitySet) -> Vec<(&'static str, &'static str)> {
    let mut cmds = vec![
        // Core — always visible
        ("/model", "Switch model"),
        ("/new", "New conversation"),
        ("/resume", "Resume conversation"),
        ("/fork", "Fork conversation"),
        ("/copy", "Copy last response"),
        ("/export", "Export conversation"),
        ("/memory", "View/edit memory"),
        ("/checkpoint", "Create checkpoint"),
        ("/settings", "Edit settings"),
        ("/help", "Show help"),
        ("/quit", "Quit"),
    ];

    if caps.is_enabled(Capability::Agentic) {
        cmds.push(("/agents", "Manage agents"));
        cmds.push(("/reflect", "Trigger reflection"));
        cmds.push(("/artifacts", "List artifacts"));
    }

    if caps.is_enabled(Capability::Mcp) {
        cmds.push(("/mcp", "MCP server status"));
    }

    if caps.is_enabled(Capability::Web) || caps.is_enabled(Capability::Desktop) {
        // These tools are available via the agent, not as slash commands,
        // but we could add admin commands here if needed.
    }

    // Always available but less prominent
    cmds.push(("/connect", "Add provider"));
    cmds.push(("/disconnect", "Remove provider"));
    cmds.push(("/providers", "List providers"));
    cmds.push(("/skills", "Manage skills"));
    cmds.push(("/clear", "Clear context"));
    cmds.push(("/mode", "Switch permission mode"));
    cmds.push(("/backend", "Switch execution backend"));

    cmds
}

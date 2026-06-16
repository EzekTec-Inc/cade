//! Capability gating helpers for REPL commands.
//!
//! When a command requires a capability that is not enabled, the helper
//! prints a user-friendly hint and returns `true` (= blocked).

use super::Repl;
use cade_core::capabilities::Capability;

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
}

//! Capability packs — each represents an optional subsystem that can be
//! enabled or disabled at runtime (and eventually at compile time via features).
//!
//! The default profile is `Full` for backward compatibility. Future releases
//! will shift the default toward `Pro` or `Core` once the gating is proven.

use std::collections::HashSet;

// region:    --- Capability

/// A single optional capability that can be toggled on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Subagents, agent messaging, reflection, artifacts, evidence
    Agentic,
    /// Screenshots, window list, desktop control, notifications
    Desktop,
    /// System tray icon
    Tray,
    /// Web search, fetch docs, browser screenshot
    Web,
    /// MCP server management and external tool schemas
    Mcp,
    /// Clipboard image paste (arboard + image crate)
    ClipboardImages,
    /// Syntax highlighting in TUI (syntect)
    SyntaxHighlighting,
    /// Advanced memory admin (tier manipulation, evidence, typed memory)
    AdvancedMemory,
    /// SDK / RPC / plugin embedding
    Integration,
}

impl Capability {
    /// All known capabilities.
    pub const ALL: &[Capability] = &[
        Capability::Agentic,
        Capability::Desktop,
        Capability::Tray,
        Capability::Web,
        Capability::Mcp,
        Capability::ClipboardImages,
        Capability::SyntaxHighlighting,
        Capability::AdvancedMemory,
        Capability::Integration,
    ];

    /// Human-readable name for display and settings.
    pub fn name(&self) -> &'static str {
        match self {
            Capability::Agentic => "agentic",
            Capability::Desktop => "desktop",
            Capability::Tray => "tray",
            Capability::Web => "web",
            Capability::Mcp => "mcp",
            Capability::ClipboardImages => "clipboard-images",
            Capability::SyntaxHighlighting => "syntax-highlighting",
            Capability::AdvancedMemory => "advanced-memory",
            Capability::Integration => "integration",
        }
    }

    /// Parse from string (case-insensitive).
    pub fn from_name(name: &str) -> Option<Capability> {
        match name.to_lowercase().replace('_', "-").as_str() {
            "agentic" => Some(Capability::Agentic),
            "desktop" => Some(Capability::Desktop),
            "tray" => Some(Capability::Tray),
            "web" => Some(Capability::Web),
            "mcp" => Some(Capability::Mcp),
            "clipboard-images" => Some(Capability::ClipboardImages),
            "syntax-highlighting" => Some(Capability::SyntaxHighlighting),
            "advanced-memory" => Some(Capability::AdvancedMemory),
            "integration" => Some(Capability::Integration),
            _ => None,
        }
    }
}

// endregion: --- Capability

// region:    --- CapabilitySet

/// An immutable set of enabled capabilities, resolved from a profile +
/// user overrides.
#[derive(Debug, Clone)]
pub struct CapabilitySet {
    enabled: HashSet<Capability>,
}

impl CapabilitySet {
    /// Create from an explicit set.
    pub fn from_caps(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            enabled: caps.into_iter().collect(),
        }
    }

    /// All capabilities enabled.
    pub fn full() -> Self {
        Self::from_caps(Capability::ALL.iter().copied())
    }

    /// Empty — only core (non-optional) tools.
    pub fn core() -> Self {
        Self {
            enabled: HashSet::new(),
        }
    }

    pub fn is_enabled(&self, cap: Capability) -> bool {
        self.enabled.contains(&cap)
    }

    pub fn enable(&mut self, cap: Capability) {
        self.enabled.insert(cap);
    }

    pub fn disable(&mut self, cap: Capability) {
        self.enabled.remove(&cap);
    }

    pub fn enabled_list(&self) -> Vec<Capability> {
        let mut v: Vec<Capability> = self.enabled.iter().copied().collect();
        v.sort_by_key(|c| c.name());
        v
    }

    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }
}

impl Default for CapabilitySet {
    /// Default = full (backward compatible).
    fn default() -> Self {
        Self::full()
    }
}

// endregion: --- CapabilitySet

// region:    --- Resolve

/// Resolve the effective capability set from a profile + optional user
/// overrides (enable/disable lists).
pub fn resolve_capabilities(enable: &[String], disable: &[String]) -> CapabilitySet {
    let mut caps = CapabilitySet::full();
    for name in enable {
        if let Some(cap) = Capability::from_name(name) {
            caps.enable(cap);
        }
    }
    for name in disable {
        if let Some(cap) = Capability::from_name(name) {
            caps.disable(cap);
        }
    }
    caps
}

// endregion: --- Resolve

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_with_overrides() {
        let caps = resolve_capabilities(&["web".to_string(), "desktop".to_string()], &[]);
        assert!(caps.is_enabled(Capability::Web));
        assert!(caps.is_enabled(Capability::Desktop));
    }

    #[test]
    fn resolve_disable_overrides_profile() {
        let caps = resolve_capabilities(&[], &["desktop".to_string(), "tray".to_string()]);
        assert!(!caps.is_enabled(Capability::Desktop));
        assert!(!caps.is_enabled(Capability::Tray));
    }

    #[test]
    fn capability_roundtrip_names() {
        for cap in Capability::ALL {
            let name = cap.name();
            let parsed = Capability::from_name(name);
            assert_eq!(parsed, Some(*cap), "roundtrip failed for {name}");
        }
    }
}

// endregion: --- Tests

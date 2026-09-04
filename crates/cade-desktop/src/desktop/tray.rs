//! Desktop system tray daemon and status coordinator (PRD #141 / Issue #145).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrayStatus {
    Idle,
    Executing,
    ApprovalRequired,
    Error,
}

impl TrayStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "CADE: Idle",
            Self::Executing => "CADE: Executing Task...",
            Self::ApprovalRequired => "CADE: Approval Required ⚠️",
            Self::Error => "CADE: Execution Error ❌",
        }
    }

    pub fn icon_emoji(&self) -> &'static str {
        match self {
            Self::Idle => "🟢",
            Self::Executing => "⚡",
            Self::ApprovalRequired => "⚠️",
            Self::Error => "❌",
        }
    }
}

/// Desktop tray state manager and background worker bridge.
#[derive(Debug, Clone)]
pub struct DesktopTrayManager {
    status: TrayStatus,
    active_agent: Option<String>,
}

impl Default for DesktopTrayManager {
    fn default() -> Self {
        Self {
            status: TrayStatus::Idle,
            active_agent: None,
        }
    }
}

impl DesktopTrayManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_status(&mut self, status: TrayStatus) {
        self.status = status;
    }

    pub fn status(&self) -> TrayStatus {
        self.status
    }

    pub fn set_active_agent(&mut self, agent: Option<String>) {
        self.active_agent = agent;
    }

    pub fn active_agent(&self) -> Option<&str> {
        self.active_agent.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_status_transitions() {
        let mut tray = DesktopTrayManager::new();
        assert_eq!(tray.status(), TrayStatus::Idle);
        assert_eq!(tray.status().icon_emoji(), "🟢");

        tray.set_status(TrayStatus::Executing);
        assert_eq!(tray.status(), TrayStatus::Executing);
        assert_eq!(tray.status().icon_emoji(), "⚡");

        tray.set_status(TrayStatus::ApprovalRequired);
        assert_eq!(tray.status(), TrayStatus::ApprovalRequired);
        assert_eq!(tray.status().icon_emoji(), "⚠️");

        tray.set_active_agent(Some("lead-architect".to_string()));
        assert_eq!(tray.active_agent(), Some("lead-architect"));
    }
}

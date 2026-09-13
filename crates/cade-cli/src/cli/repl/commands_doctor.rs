//! /doctor command handler.

use super::Repl;
use crate::Result;
use cade_core::doctor::{check_multiplexer_and_keys, MultiplexerKind};

impl Repl {
    pub(crate) async fn cmd_doctor(&mut self) -> Result<bool> {
        let report = check_multiplexer_and_keys();
        self.tui_blank();
        self.tui_hdr("  CADE System & Environment Doctor");
        self.tui_blank();

        match &report.multiplexer {
            MultiplexerKind::Tmux { socket, pane } => {
                self.tui_ok("  ✓ Multiplexer: tmux session active");
                self.tui_dim(format!("    Socket: {socket}"));
                if let Some(p) = pane {
                    self.tui_dim(format!("    Pane: {p}"));
                }
            }
            MultiplexerKind::Screen { session } => {
                self.tui_ok(format!("  ✓ Multiplexer: GNU Screen active ({session})"));
            }
            MultiplexerKind::None => {
                self.tui_ok("  ✓ Multiplexer: None (direct TTY session, no multiplexer layer)");
            }
        }

        if !report.key_passthrough_issues.is_empty() {
            self.tui_blank();
            self.tui_err("  ⚠ Key Passthrough Conflicts Detected:");
            self.tui_dim("    Un-prefixed root-table key bindings intercept characters before CADE receives them.");
            self.tui_blank();

            for issue in &report.key_passthrough_issues {
                self.tui_err(format!("    • Key '{}' intercepted:", issue.key));
                self.tui_dim(format!("      Action: {}", issue.action));
                self.tui_dim(format!("      Fix: {}", issue.remediation));
            }
            self.tui_blank();
            self.tui_dim("    After editing ~/.tmux.conf, reload with: tmux source-file ~/.tmux.conf");
        } else if matches!(report.multiplexer, MultiplexerKind::Tmux { .. }) {
            self.tui_ok("  ✓ Key Passthrough: No conflicting un-prefixed root bindings detected.");
        }

        for info in &report.info {
            self.tui_dim(format!("  ℹ {info}"));
        }
        for warn in &report.warnings {
            self.tui_err(format!("  ⚠ {warn}"));
        }

        self.tui_blank();
        Ok(false)
    }
}

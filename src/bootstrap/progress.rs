//! Startup progress bar using `indicatif::MultiProgress`.
//!
//! Provides a lightweight wrapper that tracks the various startup stages
//! (server connection, agent resolution, MCP server boot, tool registration)
//! using a shared `MultiProgress` bar.  All spinners are cleared before the
//! TUI enters the alternate screen.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

/// Tick characters for the Braille-dot spinner.
const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How often each spinner frame advances.
const TICK_INTERVAL: Duration = Duration::from_millis(80);

/// Shared multi-progress state for the entire startup sequence.
pub struct StartupProgress {
    mp: MultiProgress,
    /// `true` when we are running interactively (i.e. spinners make sense).
    enabled: bool,
}

impl StartupProgress {
    /// Create a new startup progress tracker.
    ///
    /// When `interactive` is `false` (headless / piped-stdin / --prompt),
    /// all spinner operations become no-ops so nothing is written to stderr.
    pub fn new(interactive: bool) -> Self {
        Self {
            mp: MultiProgress::new(),
            enabled: interactive,
        }
    }

    // ------------------------------------------------------------------
    // Spinner factory
    // ------------------------------------------------------------------

    /// Add a new spinner line to the multi-progress bar.
    /// Returns a `ProgressBar` handle that the caller can update or finish.
    pub fn add_spinner(&self, msg: impl Into<String>) -> ProgressBar {
        if !self.enabled {
            return ProgressBar::hidden();
        }
        let style = ProgressStyle::with_template("{spinner:.cyan} {elapsed_precise:.dim} {msg}")
            .unwrap()
            .tick_strings(SPINNER_TICKS);
        let pb = self.mp.add(ProgressBar::new_spinner());
        pb.set_style(style);
        pb.set_message(msg.into());
        pb.enable_steady_tick(TICK_INTERVAL);
        pb
    }

    // ------------------------------------------------------------------
    // Convenience helpers for common stages
    // ------------------------------------------------------------------

    /// "Connecting to cade-server…"
    pub fn start_server_connect(&self) -> ProgressBar {
        self.add_spinner("Connecting to cade-server…")
    }

    /// "Resolving agent…"
    pub fn start_agent_resolve(&self) -> ProgressBar {
        self.add_spinner("Resolving agent…")
    }


    // ------------------------------------------------------------------
    // Finish helpers
    // ------------------------------------------------------------------

    /// Mark a spinner as successfully completed with a green check.
    pub fn finish_ok(pb: &ProgressBar, msg: impl Into<String>) {
        let done_style = ProgressStyle::with_template("  {msg:.green}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner());
        pb.set_style(done_style);
        pb.finish_with_message(format!("✔ {}", msg.into()));
    }


    /// Clear the entire multi-progress display so the terminal is clean
    /// before `ratatui::init()` takes over.
    pub fn clear(&self) {
        if self.enabled {
            let _ = self.mp.clear();
        }
    }
}

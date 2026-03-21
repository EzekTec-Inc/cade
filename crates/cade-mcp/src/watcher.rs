//! File watcher for MCP settings files.
//!
//! Watches `~/.cade/settings.json`, `.cade/settings.json`, and
//! `.cade/settings.local.json` for writes.  Any change sends `()` on the
//! returned channel so the REPL can trigger a live MCP reload without
//! restarting.
//!
//! Mirrors the skill watcher in `crate::skills::spawn_skill_watcher` exactly.

use std::path::{Path, PathBuf};

/// Spawn a background thread that watches CADE settings files for changes.
/// Returns a channel the REPL polls each loop iteration (non-blocking
/// `try_recv`).  If no watchable directories exist yet the channel is
/// returned silently — the REPL never receives on it until directories appear
/// (a restart is needed in that edge case, which is acceptable).
pub fn spawn_mcp_watcher(cwd: &Path) -> tokio::sync::mpsc::Receiver<()> {
    use notify::event::ModifyKind;
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    let (tx, rx) = tokio::sync::mpsc::channel::<()>(4);

    let home = dirs::home_dir();

    // Build list of directories to watch (non-recursively).
    // We watch the parent dirs and filter by filename so we don't miss
    // atomic-write renames that some editors use.
    let mut watch_dirs: Vec<PathBuf> = Vec::new();

    if let Some(h) = &home {
        let global_dir = h.join(".cade");
        if global_dir.exists() {
            watch_dirs.push(global_dir);
        }
    }

    let project_dir = cwd.join(".cade");
    if project_dir.exists() {
        watch_dirs.push(project_dir);
    }

    if watch_dirs.is_empty() {
        return rx;
    }

    std::thread::spawn(move || {
        let (sync_tx, sync_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

        let mut watcher = match RecommendedWatcher::new(sync_tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("mcp watcher: failed to create watcher: {e}");
                return;
            }
        };

        for dir in &watch_dirs {
            if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                tracing::warn!("mcp watcher: cannot watch {}: {e}", dir.display());
            } else {
                tracing::info!("mcp watcher: watching {}", dir.display());
            }
        }

        for res in sync_rx {
            match res {
                Ok(event) => {
                    let relevant = matches!(
                        event.kind,
                        EventKind::Modify(ModifyKind::Data(_))
                            | EventKind::Modify(ModifyKind::Any)
                            | EventKind::Modify(ModifyKind::Name(_))
                    );
                    let is_settings = event.paths.iter().any(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n == "settings.json" || n == "settings.local.json")
                            .unwrap_or(false)
                    });
                    if relevant && is_settings {
                        let _ = tx.try_send(());
                    }
                }
                Err(e) => tracing::warn!("mcp watcher error: {e}"),
            }
        }
    });

    rx
}

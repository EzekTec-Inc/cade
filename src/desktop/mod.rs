pub mod capture;
pub mod control;
pub mod notify;
pub mod tray;

// Re-exports — fully wired in Phase 3
#[allow(unused_imports)]
pub use capture::ScreenCapture;
#[allow(unused_imports)]
pub use control::DesktopControl;
#[allow(unused_imports)]
pub use notify::{Urgency, notify_approval_needed, notify_task_complete, send_notification};
pub use tray::{TrayMsg, TrayStatus, spawn_tray};

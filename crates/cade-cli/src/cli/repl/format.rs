use cade_core::permissions::PermissionMode;

/// Returns (icon, label, hint) for the current permission mode.
pub(crate) fn mode_display(mode: PermissionMode) -> (&'static str, &'static str, &'static str) {
    match mode {
        PermissionMode::Plan => ("📖", "plan (read-only)", "— Use /default to resume"),
        PermissionMode::BypassPermissions => ("⚡", "yolo", "— All tools auto-approved"),
        PermissionMode::AcceptEdits => ("📝", "acceptEdits", "— File edits auto-approved"),
        PermissionMode::Default => ("✅", "default", "— Tools require approval"),
    }
}

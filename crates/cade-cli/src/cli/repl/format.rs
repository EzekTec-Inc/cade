use cade_core::permissions::PermissionMode;

/// Returns (icon, label, hint) for the current permission mode.
/// Uses user-friendly names while keeping internal mode values unchanged.
pub(crate) fn mode_display(mode: PermissionMode) -> (&'static str, &'static str, &'static str) {
    match mode {
        PermissionMode::Plan => ("📖", "Plan only", "— Read-only, no modifications. Use /mode default to resume."),
        PermissionMode::BypassPermissions => ("⚡", "Full access", "— All tools auto-approved, no confirmations."),
        PermissionMode::AcceptEdits => ("📝", "Edit freely", "— File edits auto-approved, other tools ask."),
        PermissionMode::Default => ("✅", "Safe", "— All tool calls require your approval."),
    }
}

/// Returns the internal mode name for a user-friendly label (case-insensitive).
/// Accepts both old internal names and new user-facing labels.
pub(crate) fn parse_mode_label(input: &str) -> Option<&'static str> {
    match input.to_lowercase().replace(' ', "").as_str() {
        // New user-facing labels
        "safe" => Some("default"),
        "editfreely" | "edit-freely" => Some("acceptEdits"),
        "planonly" | "plan-only" | "plan" => Some("plan"),
        "fullaccess" | "full-access" | "yolo" => Some("yolo"),
        // Old internal names (backward compat)
        "default" => Some("default"),
        "acceptedits" => Some("acceptEdits"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_display_returns_user_friendly_labels() {
        let (_, label, _) = mode_display(PermissionMode::Default);
        assert_eq!(label, "Safe");
        let (_, label, _) = mode_display(PermissionMode::BypassPermissions);
        assert_eq!(label, "Full access");
    }

    #[test]
    fn parse_mode_label_accepts_old_and_new() {
        assert_eq!(parse_mode_label("safe"), Some("default"));
        assert_eq!(parse_mode_label("Safe"), Some("default"));
        assert_eq!(parse_mode_label("yolo"), Some("yolo"));
        assert_eq!(parse_mode_label("Full access"), Some("yolo"));
        assert_eq!(parse_mode_label("default"), Some("default"));
        assert_eq!(parse_mode_label("acceptEdits"), Some("acceptEdits"));
        assert_eq!(parse_mode_label("Edit freely"), Some("acceptEdits"));
        assert_eq!(parse_mode_label("plan"), Some("plan"));
        assert_eq!(parse_mode_label("nonsense"), None);
    }
}

use crate::Result;

#[derive(Debug, Clone, Copy)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// Send a desktop OS notification with platform-appropriate urgency handling.
///
/// | Platform | Urgency mechanism                              |
/// |----------|------------------------------------------------|
/// | Linux    | D-Bus `urgency` hint + timeout                 |
/// | Windows  | Timeout → `Duration::Short` / `Duration::Long` |
/// | macOS    | Sound name (only lever available)               |
pub fn send_notification(title: &str, body: &str, urgency: Urgency) -> Result<()> {
    let mut n = notify_rust::Notification::new();
    n.appname("CADE").summary(title).body(body);

    // -- Linux: full urgency support via D-Bus hints + timeout
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let u = match urgency {
            Urgency::Low => notify_rust::Urgency::Low,
            Urgency::Normal => notify_rust::Urgency::Normal,
            Urgency::Critical => notify_rust::Urgency::Critical,
        };
        n.urgency(u);
        match urgency {
            Urgency::Low => {
                n.timeout(5_000);
            }
            Urgency::Normal => {
                n.timeout(10_000);
            }
            Urgency::Critical => {
                n.timeout(0); // persistent until dismissed
            }
        }
    }

    // -- Windows: timeout maps to winrt Duration::Short / Duration::Long
    #[cfg(target_os = "windows")]
    {
        match urgency {
            Urgency::Low => {
                n.timeout(5_000);
            }
            Urgency::Normal => {
                n.timeout(10_000);
            }
            Urgency::Critical => {
                n.timeout(0); // maps to Duration::Long via Never
            }
        }
    }

    // -- macOS: sound is the only available signal for importance.
    //    mac-notification-sys ignores both urgency hints and timeout.
    #[cfg(target_os = "macos")]
    {
        if matches!(urgency, Urgency::Critical) {
            n.sound_name("Funk");
        }
    }

    n.show().map_err(crate::Error::custom_from_err)?;

    Ok(())
}

/// Convenience: notify when a task completes
pub fn notify_task_complete(task: &str) -> Result<()> {
    send_notification("CADE — Task Complete", task, Urgency::Normal)
}

/// Convenience: notify when the agent needs approval
pub fn notify_approval_needed(tool_name: &str) -> Result<()> {
    send_notification(
        "CADE — Approval Required",
        &format!("Tool '{tool_name}' is waiting for your approval"),
        Urgency::Critical,
    )
}

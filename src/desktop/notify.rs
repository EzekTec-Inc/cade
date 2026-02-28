use anyhow::Result;

#[derive(Debug, Clone, Copy)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// Send a desktop OS notification
pub fn send_notification(title: &str, body: &str, urgency: Urgency) -> Result<()> {
    let urgency_level = match urgency {
        Urgency::Low => notify_rust::Urgency::Low,
        Urgency::Normal => notify_rust::Urgency::Normal,
        Urgency::Critical => notify_rust::Urgency::Critical,
    };

    notify_rust::Notification::new()
        .appname("CADE")
        .summary(title)
        .body(body)
        .urgency(urgency_level)
        .show()?;

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

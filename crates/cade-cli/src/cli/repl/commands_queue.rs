use super::Repl;
use super::queue_controller::ReplQueueController;
use crate::Result;

impl Repl {
    /// Manage the interactive steering and follow-up queues.
    pub(crate) async fn cmd_queue(&self, args: Option<String>) -> Result<bool> {
        let controller =
            ReplQueueController::new(self.queued_steering.clone(), self.queued_followup.clone());

        let raw_args = args.as_deref().unwrap_or("").trim();
        let mut parts = raw_args.split_whitespace();
        let action = parts.next().unwrap_or("list");

        match action {
            "" | "list" | "status" | "show" => {
                let snap = controller.snapshot();
                let output = controller.format_snapshot(&snap);
                self.tui_sys(output);
            }
            "pop" => {
                if let Some(popped) = controller.pop_followup() {
                    let remaining = controller.snapshot().total_count();
                    self.app.lock().queued_count = remaining;
                    self.tui_ok(format!(
                        "Popped most recent follow-up: \"{}\" ({remaining} remaining)",
                        popped.trim()
                    ));
                } else {
                    self.tui_sys("Follow-up queue is already empty.");
                }
            }
            "drop" | "rm" | "delete" => {
                let Some(target) = parts.next() else {
                    self.tui_sys("Usage: /queue drop <index | steering>");
                    return Ok(false);
                };

                if target.eq_ignore_ascii_case("steering") || target == "0" {
                    match controller.drop_steering() {
                        Ok(dropped) => {
                            let remaining = controller.snapshot().total_count();
                            self.app.lock().queued_count = remaining;
                            self.tui_ok(format!(
                                "Dropped pending steering: \"{}\"",
                                dropped.trim()
                            ));
                        }
                        Err(err) => self.tui_err(err.to_string()),
                    }
                } else {
                    match target.parse::<usize>() {
                        Ok(idx) => match controller.drop_followup(idx) {
                            Ok(dropped) => {
                                let remaining = controller.snapshot().total_count();
                                self.app.lock().queued_count = remaining;
                                self.tui_ok(format!(
                                    "Dropped follow-up #{idx}: \"{}\" ({remaining} remaining)",
                                    dropped.trim()
                                ));
                            }
                            Err(err) => self.tui_err(err.to_string()),
                        },
                        Err(_) => {
                            self.tui_err(format!(
                                "Invalid index '{target}'. Please specify a 1-based index (e.g. /queue drop 1) or 'steering'."
                            ));
                        }
                    }
                }
            }
            "clear" | "empty" | "reset" => {
                let (steering, count) = controller.clear();
                self.app.lock().queued_count = 0;
                let mut notices = Vec::new();
                if steering.is_some() {
                    notices.push("dropped 1 steering message".to_string());
                }
                if count > 0 {
                    notices.push(format!(
                        "dropped {count} follow-up{}",
                        if count == 1 { "" } else { "s" }
                    ));
                }

                if notices.is_empty() {
                    self.tui_sys("Queue was already empty.");
                } else {
                    self.tui_ok(format!("Cleared queue: {}.", notices.join(", ")));
                }
            }
            "help" | "-h" | "--help" => {
                self.tui_sys("Interactive Queue Commands:\n  /queue [list]             - View pending steering and follow-ups\n  /queue drop <idx|steering>- Drop an item by 1-based index or drop steering\n  /queue pop                - Remove the most recent follow-up\n  /queue clear              - Drop all queued steering and follow-ups");
            }
            unknown => {
                self.tui_err(format!(
                    "Unknown queue action '{unknown}'. Use /queue list, drop, pop, clear, or help."
                ));
            }
        }

        Ok(false)
    }
}

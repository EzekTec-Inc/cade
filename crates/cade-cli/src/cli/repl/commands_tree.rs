//! /tree command handler.

use super::Repl;
use crate::Result;
use crate::ui::ToastLevel;

impl Repl {
    pub(crate) async fn cmd_tree(&mut self) -> Result<bool> {
        let agent_id = self.agent_id();
        loop {
            match self.client.list_checkpoints(&agent_id).await {
                Err(e) => {
                    self.tui_err(format!("  ✗ list_checkpoints: {e}"));
                    break;
                }
                Ok(checkpoints) if checkpoints.is_empty() => {
                    self.tui_dim(
                        "  No checkpoints yet. Use /checkpoint [label] to create one.".to_string(),
                    );
                    break;
                }
                Ok(checkpoints) => {
                    // Show the fullscreen tree browser
                    let action = {
                        let mut app = self.app.lock();
                        let colors = app.colors.clone();
                        cade_tui::show_session_tree(&mut app.terminal, &checkpoints, &colors)
                    };
                    match action {
                        Ok(cade_tui::TreeAction::Cancel) => {
                            self.app
                                .lock()
                                .show_toast("Checkpoint browser closed", ToastLevel::Info);
                            self.tui_dim("  /tree cancelled".to_string());
                            break;
                        }
                        Ok(cade_tui::TreeAction::Delete { checkpoint_id }) => {
                            // Confirm with question
                            let title = checkpoints
                                .iter()
                                .find(|cp| cp["id"].as_str() == Some(&checkpoint_id))
                                .and_then(|cp| cp["label"].as_str())
                                .unwrap_or("(unlabelled)")
                                .to_string();
                            use crate::ui::question::{Question, QuestionOption};
                            let q = Question {
                                header: "Delete Checkpoint?".to_string(),
                                text: format!("Delete checkpoint \"{title}\"?"),
                                options: vec![
                                    QuestionOption {
                                        label: "Yes — delete".to_string(),
                                        description: String::new(),
                                    },
                                    QuestionOption {
                                        label: "No — keep".to_string(),
                                        description: String::new(),
                                    },
                                ],
                                multi_select: false,
                                allow_other: false,
                                progress: None,
                            };
                            let ans = {
                                let mut app = self.app.lock();
                                let r = app.ask_question(&q);
                                app.scroll = 0;
                                let _ = app.draw();
                                r
                            };
                            if let Ok(Some(a)) = ans
                                && a.as_str().starts_with("Yes")
                            {
                                // Drop git stash if exists
                                let stash_ref = checkpoints
                                    .iter()
                                    .find(|cp| cp["id"].as_str() == Some(&checkpoint_id))
                                    .and_then(|cp| cp["git_stash_ref"].as_str())
                                    .map(String::from);
                                if let Some(s) = stash_ref {
                                    use cade_agent::tools::git_checkpoint;
                                    let _ =
                                        git_checkpoint::delete_git_checkpoint(&s, &self.cwd).await;
                                }
                                // Delete from server
                                match self
                                    .client
                                    .delete_checkpoint(&agent_id, &checkpoint_id)
                                    .await
                                {
                                    Ok(_) => {
                                        self.app.lock().show_toast(
                                            format!("Deleted checkpoint {title}"),
                                            ToastLevel::Success,
                                        );
                                        self.tui_ok(format!("  ✓ Deleted checkpoint {title}"));
                                    }
                                    Err(e) => self
                                        .tui_err(format!("  ✗ Failed to delete checkpoint: {e}")),
                                }
                            }
                            continue;
                        }
                        Ok(cade_tui::TreeAction::Restore { checkpoint_id }) => {
                            self.tui_dim(format!("  Restoring checkpoint {checkpoint_id}…"));
                            // Find git stash ref in the checkpoint list
                            let stash_ref = checkpoints
                                .iter()
                                .find(|cp| cp["id"].as_str() == Some(&checkpoint_id))
                                .and_then(|cp| cp["git_stash_ref"].as_str())
                                .map(String::from);
                            if let Some(s) = stash_ref {
                                use cade_agent::tools::git_checkpoint;
                                match git_checkpoint::restore_git_checkpoint(&s, &self.cwd).await {
                                    Ok(()) => self.tui_ok(format!("  ✓ Git stash applied: {s}")),
                                    Err(e) => self.tui_err(format!("  ✗ Git restore: {e}")),
                                }
                            }
                            let _ = self
                                .client
                                .restore_checkpoint(&agent_id, &checkpoint_id)
                                .await;
                            self.app.lock().show_toast(
                                format!("Restored checkpoint {checkpoint_id}"),
                                ToastLevel::Success,
                            );
                            self.tui_ok(format!("  ✓ Restored to checkpoint {checkpoint_id}"));
                            break;
                        }
                        Err(e) => {
                            self.tui_err(format!("  ✗ Tree error: {e}"));
                            break;
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

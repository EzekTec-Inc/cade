//! /resume command handler.

use crate::Result;
use super::Repl;
use std::sync::Arc;
use crate::ui::RenderLine;

impl Repl {
    pub(crate) async fn cmd_resume(
        &mut self,
    ) -> Result<bool> {
            self.tui_dim("  Fetching conversations…");
            let agent_id = self.agent_id();
            match self.client.list_conversations(&agent_id).await {
                Ok(convs) => {
                    if convs.is_empty() {
                        let _ =
                            self.app
                                .lock()
                                .push(RenderLine::DimMsg(
                                "  No saved conversations yet. Use /new to start one."
                                    .to_string(),
                            ));
                    } else if let Some(picked) = self
                        .conversation_picker(Arc::clone(&self.app), &convs, &agent_id)
                        .await?
                    {
                        let cid = picked["id"].as_str().unwrap_or("").to_string();
                        *self.conversation_id.lock() =
                            Some(cid.clone());
                        { let mut s = self.session.lock();
                            let _ = s.set_conversation(Some(cid));
                        }
                        self.first_turn
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        let _ = self.app.lock().push(
                            RenderLine::SuccessMsg(format!(
                                "  ✓ Switched to: {}",
                                picked["title"].as_str().unwrap_or("(untitled)")
                            )),
                        );
                    }
                    let _ = self.app.lock().draw();
                }
                Err(e) => {
                    let _ = self
                        .app
                        .lock()
                        .push(RenderLine::ErrorMsg(e.to_string()));
                }
            }
        Ok(false)
    }

}

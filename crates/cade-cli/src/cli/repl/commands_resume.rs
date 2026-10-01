//! /resume command handler.

use super::Repl;
use crate::Result;
use crate::ui::RenderLine;
use std::sync::Arc;

impl Repl {
    pub(crate) async fn ask_and_summarize_branch(&mut self) -> Result<()> {
        let conversation_id = self.conversation_id();
        let agent_id = self.agent_id();

        // Check if the current conversation is not empty
        let has_messages = match self.client.list_conversations(&agent_id).await {
            Ok(convs) => {
                if let Some(ref cid) = conversation_id {
                    convs.iter().any(|c| c["id"].as_str() == Some(cid))
                } else {
                    false
                }
            }
            Err(_) => false,
        };

        if !has_messages {
            return Ok(());
        }

        // Prompt the user using CADE's standard confirmation modal
        use crate::ui::question::{Question, QuestionOption};
        let opts = vec![
            QuestionOption {
                label: "Yes — summarize branch".to_string(),
                description: "Consolidates recent turns and saves them to SQLite archival memory."
                    .to_string(),
            },
            QuestionOption {
                label: "No — switch immediately".to_string(),
                description: "Switches conversations without background summarization.".to_string(),
            },
        ];
        let q_widget = Question {
            header: "Branch Summary".to_string(),
            text: "Summarize this conversation before leaving?".to_string(),
            options: opts.clone(),
            multi_select: false,
            allow_other: false,
            progress: None,
        };

        let answer = self.ask_repl_question(q_widget).await?;
        let should_summarize = matches!(&answer, Some(a) if a.as_str().starts_with("Yes"));

        if should_summarize {
            self.tui_dim("  Compacting and archiving current branch...");
            let cid_ref = conversation_id.as_deref();
            match self.client.compact(&agent_id, cid_ref).await {
                Err(e) => self.tui_err(format!("  ✗ Branch summarization failed: {e}")),
                Ok(result) if result.skipped => self.tui_dim(format!(
                    "  Summarization skipped: {}",
                    result.reason.as_deref().unwrap_or("nothing to consolidate")
                )),
                Ok(result) => self.tui_ok(format!(
                    "  ✓ Branch consolidation completed ({} summary characters).",
                    result.session_summary_chars
                )),
            }
        }
        Ok(())
    }

    pub(crate) async fn cmd_resume(&mut self) -> Result<bool> {
        self.tui_dim("  Fetching conversations…");
        let agent_id = self.agent_id();
        match self.client.list_conversations(&agent_id).await {
            Ok(convs) => {
                if convs.is_empty() {
                    let _ = self.app.lock().push(RenderLine::DimMsg(
                        "  No saved conversations yet. Use /new to start one.".to_string(),
                    ));
                } else if let Some(picked) = self
                    .conversation_picker(Arc::clone(&self.app), &convs, &agent_id)
                    .await?
                {
                    // Prompt to summarize the branch being left behind!
                    let _ = self.ask_and_summarize_branch().await;

                    let cid = picked["id"].as_str().unwrap_or("").to_string();
                    if let Err(error) = self.select_conversation(Some(cid)) {
                        self.tui_err(format!("Could not switch conversation: {error}"));
                        return Ok(false);
                    }
                    self.first_turn
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    let _ = self.app.lock().push(RenderLine::SuccessMsg(format!(
                        "  ✓ Switched to: {}",
                        picked["title"].as_str().unwrap_or("(untitled)")
                    )));
                }
                let _ = self.app.lock().draw();
            }
            Err(e) => {
                let _ = self.app.lock().push(RenderLine::ErrorMsg(e.to_string()));
            }
        }
        Ok(false)
    }
}

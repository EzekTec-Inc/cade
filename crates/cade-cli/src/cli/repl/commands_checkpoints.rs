//! /checkpoints command handler.

use super::Repl;
use crate::Result;
use crate::ui::ToastLevel;

impl Repl {
    pub(crate) async fn cmd_fork(&mut self, label_arg: Option<String>) -> Result<bool> {
        // Prompt to summarize the current branch before forking!
        let _ = self.ask_and_summarize_branch().await;

        let agent_id = self.agent_id();
        let label = label_arg.as_deref().unwrap_or("fork");
        self.tui_dim(format!("  Creating fork point '{label}'…"));
        use cade_agent::tools::git_checkpoint;
        let git_cp = git_checkpoint::create_git_checkpoint(label, &self.cwd).await;
        let commit = git_cp
            .as_ref()
            .and_then(|g| g.commit_hash.as_deref())
            .map(String::from);
        // Create a checkpoint as the fork anchor
        match self
            .client
            .create_checkpoint(
                &agent_id,
                Some(label),
                Some("fork anchor"),
                self.conversation_id().as_deref(),
                commit.as_deref(),
            )
            .await
        {
            Ok(cp_id) => {
                let parent_id = self.conversation_id().unwrap_or_default();
                // Start a new conversation from this point
                match self
                    .client
                    .create_conversation_fork(&agent_id, "", &parent_id)
                    .await
                {
                    Ok(conv) => {
                        let cid = conv["id"].as_str().unwrap_or("").to_string();
                        *self.conversation_id.lock() = Some(cid.clone());
                        {
                            let mut s = self.session.lock();
                            let _ = s.set_conversation(Some(cid.clone()));
                        }
                        self.first_turn
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        self.tui_ok(format!(
                            "  ✓ Forked from checkpoint {cp_id}  →  new conversation {}",
                            &cid[..cid.len().min(16)]
                        ));
                    }
                    Err(e) => self.tui_err(format!("  ✗ Create conversation: {e}")),
                }
            }
            Err(e) => self.tui_err(format!("  ✗ Fork failed: {e}")),
        }
        Ok(false)
    }

    pub(crate) async fn cmd_checkpoint(&mut self, label_arg: Option<String>) -> Result<bool> {
        let agent_id = self.agent_id();
        let label = label_arg.as_deref().unwrap_or("manual");
        self.tui_dim(format!("  Creating checkpoint '{label}'…"));
        // Git commit if dirty
        use cade_agent::tools::git_checkpoint;
        let git_cp = git_checkpoint::create_git_checkpoint(label, &self.cwd).await;
        let commit = git_cp
            .as_ref()
            .and_then(|g| g.commit_hash.as_deref())
            .map(String::from);
        let conv_id = self.conversation_id();
        match self
            .client
            .create_checkpoint(
                &agent_id,
                Some(label),
                None,
                conv_id.as_deref(),
                commit.as_deref(),
            )
            .await
        {
            Ok(cp_id) => {
                let mut msg = format!("  ✓ Checkpoint '{label}' — ID: {cp_id}");
                if commit.is_some() {
                    msg.push_str("  (git committed)");
                }
                self.app
                    .lock()
                    .show_toast(format!("Checkpoint '{label}' created"), ToastLevel::Success);
                self.tui_ok(msg);
            }
            Err(e) => self.tui_err(format!("  ✗ Checkpoint failed: {e}")),
        }
        // -- Undo
        Ok(false)
    }
}

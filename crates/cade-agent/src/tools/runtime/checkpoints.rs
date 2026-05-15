use super::*;
use crate::tools::git_checkpoint;
use serde_json::Value;

impl ToolRuntime {
    pub(crate) async fn handle_create_checkpoint(&self, args: &Value) -> (String, bool) {
        let label = args["label"]
            .as_str()
            .unwrap_or("checkpoint")
            .trim()
            .to_string();
        let description = args["description"].as_str().map(String::from);

        // 1. Attempt a git commit if dirty
        let git_cp = git_checkpoint::create_git_checkpoint(&label, &self.cwd).await;
        let commit_hash = git_cp
            .as_ref()
            .and_then(|g| g.commit_hash.as_deref())
            .map(String::from);

        // 2. Create server-side checkpoint record
        let conv_id = self.conversation_id.as_deref();
        match self
            .storage
            .create_checkpoint(
                &self.agent_id,
                conv_id,
                Some("main"),
                Some(&label),
                description.as_deref(),
                commit_hash.as_deref(),
            )
            .await
        {
            Ok(cp_id) => {
                let mut msg = format!("Checkpoint '{label}' created. ID: {cp_id}");
                if let Some(h) = &commit_hash {
                    msg.push_str(&format!("\nHEAD: {}", &h[..8.min(h.len())]));
                }
                (msg, false)
            }
            Err(e) => (format!("Failed to create checkpoint: {e}"), true),
        }
    }

    pub(crate) async fn handle_restore_checkpoint(&self, args: &Value) -> (String, bool) {
        let cp_id = args["checkpoint_id"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if cp_id.is_empty() {
            return ("Error: 'checkpoint_id' is required".to_string(), true);
        }

        // Get the checkpoint to find git info
        let cp = match self.storage.get_checkpoint(&self.agent_id, &cp_id).await {
            Ok(v) => v,
            Err(e) => return (format!("Checkpoint not found: {e}"), true),
        };

        // Reset to git commit if there is one
        let commit_hash = cp["git_commit_hash"].as_str().unwrap_or("").to_string();
        if !commit_hash.is_empty()
            && let Err(e) = git_checkpoint::restore_git_checkpoint(&commit_hash, &self.cwd).await
        {
            return (format!("Git reset failed: {e}"), true);
        }

        // Mark checkpoint as restored on server
        if let Err(e) = self
            .storage
            .restore_checkpoint(&self.agent_id, &cp_id)
            .await
        {
            tracing::warn!("restore_checkpoint server update failed: {e}");
        }

        let label = cp["label"].as_str().unwrap_or("?");
        (
            format!("Restored to checkpoint '{label}' ({cp_id})."),
            false,
        )
    }
}

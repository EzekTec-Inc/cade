use super::*;
use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;
use cade_core::skills::discover_all_skills;
use cade_core::tool_ids::*;
use crate::agent::client::HttpTransport;
use crate::backends::{ExecutionBackend, LocalBackend};
use crate::mcp::McpManager;
use crate::tools::git_checkpoint;
use crate::tools::{dispatch, memory};

impl ToolRuntime {
    async fn handle_create_checkpoint(&self, args: &Value) -> (String, bool) {
        let label = args["label"]
            .as_str()
            .unwrap_or("checkpoint")
            .trim()
            .to_string();
        let description = args["description"].as_str().map(String::from);

        // 1. Attempt a git stash
        let git_cp = git_checkpoint::create_git_checkpoint(&label, &self.cwd).await;
        let stash_ref = git_cp
            .as_ref()
            .and_then(|g| g.stash_ref.as_deref())
            .map(String::from);
        let commit_hash = git_cp
            .as_ref()
            .and_then(|g| g.commit_hash.as_deref())
            .map(String::from);

        // 2. Create server-side checkpoint record
        let conv_id = self.conversation_id.as_deref();
        match self
            .client
            .create_checkpoint(
                &self.agent_id,
                Some(&label),
                description.as_deref(),
                conv_id,
                stash_ref.as_deref(),
                commit_hash.as_deref(),
            )
            .await
        {
            Ok(cp_id) => {
                let mut msg = format!("Checkpoint '{label}' created. ID: {cp_id}");
                if let Some(s) = &stash_ref {
                    msg.push_str(&format!("\nGit stash: {s}"));
                }
                if let Some(h) = &commit_hash {
                    msg.push_str(&format!("\nHEAD: {}", &h[..8.min(h.len())]));
                }
                (msg, false)
            }
            Err(e) => (format!("Failed to create checkpoint: {e}"), true),
        }
    }

    async fn handle_restore_checkpoint(&self, args: &Value) -> (String, bool) {
        let cp_id = args["checkpoint_id"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if cp_id.is_empty() {
            return ("Error: 'checkpoint_id' is required".to_string(), true);
        }

        // Get the checkpoint to find git info
        let cp = match self.client.get_checkpoint(&self.agent_id, &cp_id).await {
            Ok(v) => v,
            Err(e) => return (format!("Checkpoint not found: {e}"), true),
        };

        // Apply git stash if there is one
        let stash_ref = cp["git_stash_ref"].as_str().unwrap_or("").to_string();
        if !stash_ref.is_empty()
            && let Err(e) = git_checkpoint::restore_git_checkpoint(&stash_ref, &self.cwd).await
        {
            return (format!("Git restore failed: {e}"), true);
        }

        // Mark checkpoint as restored on server
        if let Err(e) = self.client.restore_checkpoint(&self.agent_id, &cp_id).await {
            tracing::warn!("restore_checkpoint server update failed: {e}");
        }

        let label = cp["label"].as_str().unwrap_or("?");
        (
            format!("Restored to checkpoint '{label}' ({cp_id})."),
            false,
        )
    }

}

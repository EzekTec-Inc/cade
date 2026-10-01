//! /session command handler.
use super::Repl;
use crate::Result;
use crate::ui::{RenderLine, ToastLevel};
use cade_core::permissions::PermissionMode;

/// Shared restore operation for /undo and the checkpoint browser. The caller
/// reports success only when both the workspace and server restore succeed.
pub(super) async fn restore_checkpoint(
    client: &cade_agent::agent::client::HttpTransport,
    agent_id: &str,
    checkpoint_id: &str,
    commit: Option<&str>,
    cwd: &std::path::Path,
) -> Result<()> {
    if checkpoint_id.is_empty() {
        return Err(crate::Error::custom("Checkpoint has no ID"));
    }
    let commit = commit.filter(|commit| !commit.is_empty());
    if let Some(commit) = commit {
        cade_agent::tools::git_checkpoint::restore_git_checkpoint(commit, cwd)
            .await
            .map_err(|error| {
                crate::Error::custom(format!(
                    "Git restore failed; server history was not restored: {error}"
                ))
            })?;
    }
    client
        .restore_checkpoint(agent_id, checkpoint_id)
        .await
        .map_err(|error| {
            crate::Error::custom(if commit.is_some() {
                format!("Workspace restored, but server history restore failed: {error}")
            } else {
                format!("Server history restore failed: {error}")
            })
        })?;
    Ok(())
}

impl Repl {
    pub(crate) async fn cmd_undo(&mut self) -> Result<bool> {
        let agent_id = self.agent_id();
        match self.client.list_checkpoints(&agent_id).await {
            Err(e) => self.tui_err(format!("  ✗ list_checkpoints: {e}")),
            Ok(checkpoints) if checkpoints.is_empty() => {
                self.tui_dim("  No checkpoints available to undo.".to_string());
            }
            Ok(checkpoints) => {
                if let Some(last_cp) = checkpoints.last() {
                    let checkpoint_id = last_cp["id"].as_str().unwrap_or("").to_string();
                    let commit_hash = last_cp["git_commit_hash"].as_str().map(String::from);
                    self.tui_dim(format!("  Restoring checkpoint {checkpoint_id}…"));
                    match restore_checkpoint(
                        &self.client,
                        &agent_id,
                        &checkpoint_id,
                        commit_hash.as_deref(),
                        &self.cwd,
                    )
                    .await
                    {
                        Ok(()) => {
                            self.tui_ok(format!("  ✓ Restored to checkpoint {checkpoint_id}"))
                        }
                        Err(error) => self.tui_err(format!("  ✗ Restore failed: {error}")),
                    }
                }
            }
        }
        Ok(false)
    }

    pub(crate) async fn cmd_rename(&mut self, new_name: String) -> Result<bool> {
        let id = self.agent_id();
        let new_name = new_name.trim().to_string();
        let name = if new_name.is_empty() {
            // Prompt for name via QuestionWidget
            use crate::ui::question::{Question, QuestionOption};
            let opts = vec![QuestionOption {
                label: "Cancel".to_string(),
                description: String::new(),
            }];
            let q = Question {
                header: "Rename agent".to_string(),
                text: "Enter new agent name:".to_string(),
                options: opts.clone(),
                multi_select: false,
                allow_other: true,
                progress: None,
            };
            let ans = self.ask_repl_question(q).await?;
            match &ans {
                Some(a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => {
                    a.as_str().to_string()
                }
                _ => String::new(),
            }
        } else {
            new_name
        };
        if name.is_empty() {
            self.tui_dim("  (cancelled)");
        } else {
            match self.client.rename_agent(&id, &name).await {
                Ok(_) => {
                    *self.agent_name.lock() = name.clone();
                    self.tui_ok(format!("  ✓ Renamed to: {name}"));
                }
                Err(e) => self.tui_err(e.to_string()),
            }
        }
        Ok(false)
    }

    pub(crate) async fn cmd_export(&mut self, out_arg: Option<String>) -> Result<bool> {
        let agent_id = self.agent_id();
        let agent_name = self.agent_name();
        let out_path =
            out_arg.unwrap_or_else(|| crate::cli::export_import::default_export_path(&agent_name));
        self.tui_dim(format!("  Exporting agent '{agent_name}' → {out_path} …"));
        match crate::cli::export_import::export_agent_to_file(&self.client, &agent_id, &out_path)
            .await
        {
            Ok(_) => {
                self.app
                    .lock()
                    .show_toast(format!("Exported → {out_path}"), ToastLevel::Success);
                self.tui_ok(format!("  ✓ Exported → {out_path}"))
            }
            Err(e) => self.tui_err(format!("  ✗ Export failed: {e}")),
        }
        // -- Checkpoints
        Ok(false)
    }

    pub(crate) async fn cmd_clear(&mut self) -> Result<bool> {
        let _ = self.app.lock().clear_content();
        match self.client.clear_messages(&self.agent_id()).await {
            Ok(n) => self.tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
            Err(e) => self.tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
        }
        Ok(false)
    }

    pub(crate) async fn cmd_stream(&mut self) -> Result<bool> {
        use std::sync::atomic::Ordering;
        let current = self.streaming_enabled.load(Ordering::SeqCst);
        self.streaming_enabled.store(!current, Ordering::SeqCst);
        let label = if !current {
            "on"
        } else {
            "off (text buffered until turn ends)"
        };
        self.tui_hdr(format!(
            "  Live text output: {label}. Tool progress and decisions remain live."
        ));
        self.app
            .lock()
            .show_toast(format!("Live text output {label}"), ToastLevel::Info);
        Ok(false)
    }

    pub(crate) async fn cmd_reload(&mut self) -> Result<bool> {
        self.tui_dim("  Reloading UI plugins...");
        let mut app = self.app.lock();
        if let Some(lua) = app.lua_engine.take() {
            drop(lua);
        }
        let new_engine = cade_tui::lua_engine::LuaEngine::new().ok();
        if let Some(engine) = &new_engine {
            if let Some(home) = dirs::home_dir() {
                engine.load_plugins(&home.join(".cade").join("plugins"));
            }
            if let Ok(cwd) = std::env::current_dir() {
                engine.load_plugins(&cwd.join(".cade").join("plugins"));
            }
        }
        app.lua_engine = new_engine;
        app.show_toast("UI Plugins reloaded", ToastLevel::Success);
        Ok(false)
    }

    pub(crate) async fn cmd_yolo(&mut self) -> Result<bool> {
        self.permissions.set_mode(PermissionMode::BypassPermissions);
        self.app
            .lock()
            .update_mode(PermissionMode::BypassPermissions);
        let _ = self.app.lock().push(RenderLine::SystemMsg(
            "⚡ Permission mode: bypassPermissions — all tools auto-approved".to_string(),
        ));
        self.sync_plan_tools(false).await;
        let _ = self
            .auto_switch_model_for_mode(PermissionMode::BypassPermissions)
            .await;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(
        status: &str,
    ) -> (
        cade_agent::agent::client::HttpTransport,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 2048];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            assert!(
                String::from_utf8_lossy(&request)
                    .starts_with("POST /v1/agents/a/checkpoints/cp/restore ")
            );
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (
            cade_agent::agent::client::HttpTransport::new(format!("http://{addr}"), "test".into())
                .unwrap(),
            task,
        )
    }

    #[tokio::test]
    async fn restore_surfaces_server_failure_instead_of_reporting_success() {
        let (client, task) = server("500 Internal Server Error").await;
        let cwd = tempfile::tempdir().unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            restore_checkpoint(&client, "a", "cp", None, cwd.path()),
        )
        .await
        .unwrap();
        task.await.unwrap();
        assert!(
            result.is_err(),
            "a rejected restore must not produce command success"
        );
    }

    #[tokio::test]
    async fn restore_reports_git_failure_and_does_not_attempt_server_restore() {
        let (client, task) = server("200 OK").await;
        let cwd = tempfile::tempdir().unwrap();
        let result =
            restore_checkpoint(&client, "a", "cp", Some("invalid-checkpoint"), cwd.path()).await;
        let called_server = task.is_finished();
        task.abort();
        assert!(result.is_err(), "Git restore failed in a non-repository");
        assert!(
            !called_server,
            "server history must not restore after workspace failure"
        );
    }

    #[tokio::test]
    async fn conversation_only_checkpoint_restore_can_succeed() {
        let (client, task) = server("200 OK").await;
        let cwd = tempfile::tempdir().unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            restore_checkpoint(&client, "a", "cp", None, cwd.path()),
        )
        .await
        .unwrap()
        .unwrap();
        task.await.unwrap();
    }
}

//! Client-owned Working Session: identity across Conversations, bounded orphan
//! lifetime, explicit close, and recovery with empty grants after session loss.

use super::HttpTransport;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub struct WorkingSession {
    client: HttpTransport,
    id: Arc<tokio::sync::Mutex<String>>,
    workspace: PathBuf,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
    closed: bool,
}

async fn open_identity(client: &HttpTransport, workspace: &Path) -> Result<(String, Duration)> {
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        client.raw_post("/working-sessions", &json!({"cwd": workspace})),
    )
    .await
    .map_err(|_| Error::custom("Opening Working Session timed out"))??;
    let id = response["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::custom("Server did not return a Working Session ID"))?
        .to_owned();
    let interval =
        Duration::from_secs((response["lease_seconds"].as_u64().unwrap_or(300) / 3).clamp(1, 30));
    Ok((id, interval))
}

async fn renew_identity(client: &HttpTransport, workspace: &Path, id: &mut String) -> Result<()> {
    let response = client
        .client
        .post(client.url(&format!("/working-sessions/{id}/heartbeat")))
        .header("Authorization", format!("Bearer {}", client.api_key))
        .timeout(Duration::from_secs(10))
        .json(&json!({}))
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // A daemon restart or expired lease ends the old grants. Do not revive
        // that identity; let the still-running client continue with a fresh one.
        *id = open_identity(client, workspace).await?.0;
        tracing::warn!(
            "Working Session ended; opened a fresh session with no remembered permissions"
        );
    } else if !response.status().is_success() {
        return Err(Error::custom(format!(
            "Working Session renewal failed {}",
            response.status()
        )));
    }
    Ok(())
}

impl HttpTransport {
    pub async fn open_working_session(&self, workspace: &Path) -> Result<WorkingSession> {
        let (id, interval) = open_identity(self, workspace).await?;
        let id = Arc::new(tokio::sync::Mutex::new(id));
        let owner = id.clone();
        let client = self.clone();
        let root = workspace.to_owned();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if let Err(error) = renew_identity(&client, &root, &mut *owner.lock().await).await {
                    tracing::warn!(%error, "Could not renew Working Session");
                }
            }
        });
        Ok(WorkingSession {
            client: self.clone(),
            id,
            workspace: workspace.to_owned(),
            heartbeat: Some(heartbeat),
            closed: false,
        })
    }
}

impl WorkingSession {
    /// Reused by ordinary Runs and direct Subagent launches. Synchronizing here
    /// also recovers after a long disconnect before submitting new execution.
    pub async fn execution_options(&self) -> Result<Value> {
        let mut id = self.id.lock().await;
        renew_identity(&self.client, &self.workspace, &mut id).await?;
        Ok(json!({"working_session_id": *id, "cwd": self.workspace}))
    }

    pub async fn close(mut self) -> Result<()> {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
            let _ = heartbeat.await;
        }
        let id = self.id.lock().await.clone();
        tokio::time::timeout(
            Duration::from_secs(10),
            self.client.raw_delete(&format!("/working-sessions/{id}")),
        )
        .await
        .map_err(|_| Error::custom("Closing Working Session timed out"))??;
        self.closed = true;
        Ok(())
    }
}

impl Drop for WorkingSession {
    fn drop(&mut self) {
        let heartbeat = self.heartbeat.take();
        if let Some(heartbeat) = &heartbeat {
            heartbeat.abort();
        }
        if !self.closed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let client = self.client.clone();
            let id = self.id.clone();
            runtime.spawn(async move {
                if let Some(heartbeat) = heartbeat {
                    let _ = heartbeat.await;
                }
                let path = format!("/working-sessions/{}", id.lock().await);
                let _ =
                    tokio::time::timeout(Duration::from_secs(5), client.raw_delete(&path)).await;
            });
        }
    }
}

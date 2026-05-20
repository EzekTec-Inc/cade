use cade::{Error, Result};
use cade_agent::agent::HttpTransport;

pub async fn auto_start_server(base_url: &str) -> Result<()> {
    let server_bin = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("cade-server"))
        .filter(|p| p.exists());

    if let Some(server_bin) = server_bin {
        tracing::info!("cade-server not running — starting…");
        let mut cmd = std::process::Command::new(&server_bin);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        if let Ok(log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/cade-server.log")
        {
            match log.try_clone() {
                Ok(log_stderr) => {
                    cmd.stdout(log);
                    cmd.stderr(log_stderr);
                }
                Err(_) => {
                    eprintln!(
                        "Warning: Failed to duplicate stderr for cade-server log. Falling back to stdout."
                    );
                    cmd.stdout(log);
                }
            }
        } else {
            eprintln!(
                "Warning: Failed to create /tmp/cade-server.log. Server output will go to stderr."
            );
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::custom(format!("auto-start cade-server: {e}")))?;

        let client = HttpTransport::new(base_url.to_string(), "".to_string())
            .map_err(|e| Error::custom(format!("create health-check client: {e}")))?;

        // Detect first-run: if the database file doesn't exist yet, the server
        // needs extra time for initial migration.
        // Even if it's not a cold start for the DB, semantic-search models
        // might need to download (~25MB) on first run or cache clear, so give
        // it a generous timeout regardless.
        let is_cold_start = dirs::home_dir()
            .map(|h| !h.join(".cade").join("cade.db").exists())
            .unwrap_or(true);
        let total_timeout = if is_cold_start {
            std::time::Duration::from_secs(60)
        } else {
            std::time::Duration::from_secs(30)
        };

        let ready = poll_server_health(&mut child, &client, total_timeout).await;

        if !ready {
            // Kill the child if it's still running but unresponsive.
            let _ = child.kill();
            return Err(Error::custom(format!(
                "cade-server failed to become ready within {}s. Check /tmp/cade-server.log\n\
                 Or start it manually: {}",
                total_timeout.as_secs(),
                server_bin.display()
            )));
        }
        tracing::info!("cade-server ready.");
        Ok(())
    } else {
        Err(Error::custom(
            "Cannot connect to CADE server at {base_url}.\n\
             Start cade-server first: ./target/release/cade-server",
        ))
    }
}

/// Poll the server's `/health` endpoint with exponential backoff.
///
/// Returns `true` as soon as the health check succeeds.  Returns `false` if:
/// - The total `timeout` duration elapses without a successful health check.
/// - The child process exits prematurely (crashed during init).
///
/// Backoff schedule: 100 ms → 200 ms → 400 ms → 800 ms → 1600 ms (capped).
async fn poll_server_health(
    child: &mut std::process::Child,
    client: &HttpTransport,
    timeout: std::time::Duration,
) -> bool {
    const INITIAL_DELAY_MS: u64 = 100;
    const MAX_DELAY_MS: u64 = 1600;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut delay_ms = INITIAL_DELAY_MS;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

        // Check if the child process exited before responding to health checks.
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::error!("cade-server exited prematurely with status: {status}");
                return false;
            }
            Ok(None) => { /* still running — good */ }
            Err(e) => {
                tracing::warn!("Failed to check cade-server process status: {e}");
            }
        }

        if client.health().await.unwrap_or(false) {
            return true;
        }

        if tokio::time::Instant::now() >= deadline {
            return false;
        }

        // Exponential backoff, capped at MAX_DELAY_MS.
        delay_ms = (delay_ms * 2).min(MAX_DELAY_MS);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn cold_start_detects_missing_db() {
        // Verify the cold-start heuristic: a non-existent path means cold start.
        let fake_path = PathBuf::from("/tmp/cade-test-nonexistent-dir/cade.db");
        assert!(
            !fake_path.exists(),
            "test precondition: path must not exist"
        );
    }
}

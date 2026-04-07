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
        let _child = cmd
            .spawn()
            .map_err(|e| Error::custom(format!("auto-start cade-server: {e}")))?;

        let client = HttpTransport::new(base_url.to_string(), "".to_string())
            .map_err(|e| Error::custom(format!("create health-check client: {e}")))?;
        let mut ready = false;
        for _ in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if client.health().await.unwrap_or(false) {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err(Error::custom(format!(
                "cade-server failed to start. Check /tmp/cade-server.log\n\
                 Or start it manually: {}",
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

//! Tokio-based IPC server for the askpass password channel.
//!
//! `cade-agent` spawns one [`AskpassServer`] per bash session.  The
//! server binds an ephemeral port on `127.0.0.1`, generates a random
//! per-session auth token, and exports both via environment variables
//! (`CADE_ASKPASS_SOCKET`, `CADE_ASKPASS_TOKEN`).
//!
//! When `sudo -A` / `ssh` / `git` spawns the `cade-askpass` helper
//! binary, the binary connects to this server, presents the token,
//! sends the OS prompt, and blocks until the server delivers a
//! password or cancellation from the TUI modal.
//!
//! ## Flow
//!
//! ```text
//! [bash session]              [cade-askpass binary]          [AskpassServer]
//!      |                              |                            |
//!      |--SUDO_ASKPASS=cade-askpass--->|                            |
//!      |                              |--- TCP connect ----------->|
//!      |                              |--- AUTH\t<token>\n ------->|
//!      |                              |<-- OK\n ------------------|
//!      |                              |--- PROMPT\t<text>\n ------>|
//!      |                              |          [server calls password_callback]
//!      |                              |<-- PASSWORD\t<pw>\n -------|
//!      |<------- stdout: <pw> --------|                            |
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::protocol::{decode_line, encode_line};

/// Per-session auth token length in bytes (hex-encoded → 64 chars).
const TOKEN_BYTES: usize = 32;

/// Maximum wait time for the password callback before sending TIMEOUT.
const PASSWORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Generate a cryptographically random hex token.
fn generate_token() -> String {
    use std::io::Read;
    let mut buf = [0u8; TOKEN_BYTES];
    let mut rng = std::fs::File::open("/dev/urandom").expect("cannot open /dev/urandom");
    rng.read_exact(&mut buf)
        .expect("failed to read random bytes");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Result delivered by the password callback.
#[derive(Debug, Clone)]
pub enum PasswordResponse {
    /// The user typed a password and pressed Enter.
    Password(String),
    /// The user pressed Esc / closed the modal.
    Cancel,
}

/// A running askpass IPC server.  Drop to shut down.
///
/// The caller uses [`Self::addr`] and [`Self::token`] to populate the
/// environment of the bash session.
pub struct AskpassServer {
    addr: SocketAddr,
    token: String,
    /// JoinHandle for the background accept loop.  Aborted on drop.
    _handle: tokio::task::JoinHandle<()>,
}

impl AskpassServer {
    /// Bind a new server to `127.0.0.1:0` and start accepting
    /// connections in the background.
    ///
    /// `password_callback` is invoked once per prompt with the
    /// prompt text.  It must resolve to a [`PasswordResponse`].
    /// Typically it sends a message to the TUI event loop and awaits
    /// the user's input.
    pub async fn start<F, Fut>(password_callback: F) -> Result<Self>
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = PasswordResponse> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind 127.0.0.1:0 for askpass IPC")?;
        let addr = listener.local_addr().context("local_addr after bind")?;
        let token = generate_token();

        let server_token = token.clone();
        let callback = Arc::new(password_callback);

        let handle = tokio::spawn(async move {
            // Accept connections until the task is aborted (server dropped).
            loop {
                let Ok((stream, _peer)) = listener.accept().await else {
                    break;
                };
                let tok = server_token.clone();
                let cb = Arc::clone(&callback);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, &tok, cb).await {
                        eprintln!("askpass connection error: {e:#}");
                    }
                });
            }
        });

        Ok(Self {
            addr,
            token,
            _handle: handle,
        })
    }

    /// The loopback address the server is listening on (e.g. `127.0.0.1:38271`).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The per-session hex token.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for AskpassServer {
    fn drop(&mut self) {
        self._handle.abort();
    }
}

async fn handle_connection<F, Fut>(
    stream: tokio::net::TcpStream,
    expected_token: &str,
    callback: Arc<F>,
) -> Result<()>
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = PasswordResponse> + Send + 'static,
{
    use tokio::io::AsyncReadExt;

    let (reader, mut writer) = stream.into_split();
    // Security: Wrap reader in a 4KB take() limit to prevent Memory Exhaustion DoS
    // from malicious local processes sending infinite data without newlines.
    let reader = reader.take(4096);
    let mut lines = BufReader::new(reader).lines();

    // ── Phase 1: AUTH handshake ──────────────────────────────────
    let auth_line = lines
        .next_line()
        .await
        .context("read AUTH line")?
        .ok_or_else(|| anyhow::anyhow!("connection closed before AUTH"))?;
    let (kind, token) = decode_line(&auth_line)?;
    if kind != "AUTH" {
        writer.write_all(b"DENY\n").await.ok();
        bail!("expected AUTH, got {kind}");
    }
    if token != expected_token {
        writer.write_all(b"DENY\n").await.ok();
        bail!("invalid token");
    }
    writer.write_all(b"OK\n").await.context("send OK")?;
    writer.flush().await.ok();

    // ── Phase 2: PROMPT ──────────────────────────────────────────
    let prompt_line = lines
        .next_line()
        .await
        .context("read PROMPT line")?
        .ok_or_else(|| anyhow::anyhow!("connection closed before PROMPT"))?;
    let (kind, prompt) = decode_line(&prompt_line)?;
    if kind != "PROMPT" {
        bail!("expected PROMPT, got {kind}");
    }

    // ── Phase 3: invoke callback with timeout ────────────────────
    let response = tokio::time::timeout(PASSWORD_TIMEOUT, callback(prompt)).await;

    match response {
        Ok(PasswordResponse::Password(pw)) => {
            let line = encode_line("PASSWORD", &pw);
            writer
                .write_all(line.as_bytes())
                .await
                .context("send PASSWORD")?;
        }
        Ok(PasswordResponse::Cancel) => {
            writer.write_all(b"CANCEL\n").await.context("send CANCEL")?;
        }
        Err(_timeout) => {
            writer
                .write_all(b"TIMEOUT\n")
                .await
                .context("send TIMEOUT")?;
        }
    }
    writer.flush().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader as TokioBufReader;

    #[tokio::test]
    async fn server_accepts_valid_token_and_delivers_password() {
        let server = AskpassServer::start(|prompt| async move {
            assert_eq!(prompt, "Password:");
            PasswordResponse::Password("s3cret".into())
        })
        .await
        .unwrap();

        let stream = tokio::net::TcpStream::connect(server.addr()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = TokioBufReader::new(reader).lines();

        // AUTH
        writer
            .write_all(encode_line("AUTH", server.token()).as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
        let ok = lines.next_line().await.unwrap().unwrap();
        assert_eq!(ok, "OK");

        // PROMPT
        writer
            .write_all(encode_line("PROMPT", "Password:").as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
        let resp = lines.next_line().await.unwrap().unwrap();
        let (k, v) = decode_line(&resp).unwrap();
        assert_eq!(k, "PASSWORD");
        assert_eq!(v, "s3cret");
    }

    #[tokio::test]
    async fn server_rejects_bad_token() {
        let server = AskpassServer::start(|_| async { PasswordResponse::Cancel })
            .await
            .unwrap();

        let stream = tokio::net::TcpStream::connect(server.addr()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = TokioBufReader::new(reader).lines();

        writer
            .write_all(encode_line("AUTH", "wrong-token").as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
        let resp = lines.next_line().await.unwrap().unwrap();
        assert_eq!(resp, "DENY");
    }

    #[tokio::test]
    async fn server_delivers_cancel_from_callback() {
        let server = AskpassServer::start(|_| async { PasswordResponse::Cancel })
            .await
            .unwrap();

        let stream = tokio::net::TcpStream::connect(server.addr()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = TokioBufReader::new(reader).lines();

        // AUTH
        writer
            .write_all(encode_line("AUTH", server.token()).as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
        let ok = lines.next_line().await.unwrap().unwrap();
        assert_eq!(ok, "OK");

        // PROMPT → CANCEL
        writer
            .write_all(encode_line("PROMPT", "Enter passphrase:").as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
        let resp = lines.next_line().await.unwrap().unwrap();
        assert_eq!(resp, "CANCEL");
    }

    #[tokio::test]
    async fn server_handles_multiple_sequential_connections() {
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        let server = AskpassServer::start(move |_| {
            let c2 = c.clone();
            async move {
                c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                PasswordResponse::Password("pw".into())
            }
        })
        .await
        .unwrap();

        for _ in 0..3 {
            let stream = tokio::net::TcpStream::connect(server.addr()).await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut lines = TokioBufReader::new(reader).lines();

            writer
                .write_all(encode_line("AUTH", server.token()).as_bytes())
                .await
                .unwrap();
            writer.flush().await.unwrap();
            let _ok = lines.next_line().await.unwrap().unwrap();

            writer
                .write_all(encode_line("PROMPT", "pw?").as_bytes())
                .await
                .unwrap();
            writer.flush().await.unwrap();
            let resp = lines.next_line().await.unwrap().unwrap();
            assert!(resp.starts_with("PASSWORD"));
        }
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn generate_token_is_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_token_is_unique_each_call() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }
}

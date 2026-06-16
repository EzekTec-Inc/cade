//! `cade-askpass` — the Askpass helper binary.
//!
//! Invoked by `sudo -A`, `ssh`, or `git` (via `SUDO_ASKPASS` /
//! `SSH_ASKPASS` / `GIT_ASKPASS`) when a password prompt is needed
//! for a process spawned by the CADE bash tool.
//!
//! See [`cade_askpass::protocol`] for the wire format documentation.
//!
//! ## Security notes
//!
//! - The binary authenticates to the IPC server via a per-session
//!   token stored in `CADE_ASKPASS_TOKEN`.  Connections with a bad or
//!   missing token are rejected (`DENY`).
//! - The password is held in memory as a `String` for as long as it
//!   takes to write to stdout.  We make no attempt to mlock or
//!   zeroize — that is the responsibility of the calling utility.
//! - We never log the prompt or password.  The only stderr output is
//!   error context with no sensitive content.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use anyhow::{Context, Result, anyhow, bail};

use cade_askpass::protocol::Message;
use cade_askpass::{ENV_SOCKET, ENV_TOKEN};

const PROTOCOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn main() {
    if let Err(e) = run() {
        eprintln!("cade-askpass: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("missing prompt argument"))?;
    let socket = std::env::var(ENV_SOCKET)
        .with_context(|| format!("environment variable {ENV_SOCKET} not set"))?;
    let token = std::env::var(ENV_TOKEN)
        .with_context(|| format!("environment variable {ENV_TOKEN} not set"))?;

    let mut stream = TcpStream::connect(&socket)
        .with_context(|| format!("connecting to askpass server at {socket}"))?;
    stream
        .set_read_timeout(Some(PROTOCOL_TIMEOUT))
        .context("set read timeout")?;
    stream
        .set_write_timeout(Some(PROTOCOL_TIMEOUT))
        .context("set write timeout")?;

    // ── Phase 1: AUTH handshake ──────────────────────────────────
    let auth_msg = Message::Auth(token);
    stream
        .write_all(auth_msg.encode().as_bytes())
        .context("send AUTH to askpass server")?;
    stream.flush().ok();

    // Security: Limit reader to 4KB to prevent infinite streaming exhaustions.
    let limit_reader = stream.try_clone().context("clone stream")?.take(4096);
    let mut reader = BufReader::new(limit_reader);
    let mut auth_response = String::new();
    reader
        .read_line(&mut auth_response)
        .context("read auth response")?;

    let msg =
        Message::decode(auth_response.trim_end_matches('\n')).context("decode auth response")?;
    match msg {
        Message::Ok => {}
        Message::Deny => bail!("server rejected auth token"),
        other => bail!("unexpected auth response: {other:?}"),
    }

    // ── Phase 2: PROMPT ──────────────────────────────────────────
    let prompt_msg = Message::Prompt(prompt);
    stream
        .write_all(prompt_msg.encode().as_bytes())
        .context("send prompt to askpass server")?;
    stream.flush().ok();

    let mut response = String::new();
    reader
        .read_line(&mut response)
        .context("read response from askpass server")?;

    let msg = Message::decode(response.trim_end_matches('\n')).context("decode prompt response")?;
    match msg {
        Message::Password(value) => {
            print!("{value}");
            std::io::stdout().flush().ok();
            Ok(())
        }
        Message::Cancel => bail!("user cancelled password prompt"),
        Message::Timeout => bail!("password prompt timed out"),
        other => bail!("unexpected response kind: {other:?}"),
    }
}

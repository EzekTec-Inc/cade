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

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use anyhow::{Context, Result, anyhow, bail};

use cade_askpass::protocol::{decode_line, encode_line};
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
    let auth_line = encode_line("AUTH", &token);
    stream
        .write_all(auth_line.as_bytes())
        .context("send AUTH to askpass server")?;
    stream.flush().ok();

    let mut reader = BufReader::new(stream.try_clone().context("clone stream")?);
    let mut auth_response = String::new();
    reader
        .read_line(&mut auth_response)
        .context("read auth response")?;

    let (kind, _) = decode_line(auth_response.trim_end_matches('\n'))?;
    match kind.as_str() {
        "OK" => {}
        "DENY" => bail!("server rejected auth token"),
        other => bail!("unexpected auth response '{other}'"),
    }

    // ── Phase 2: PROMPT ──────────────────────────────────────────
    let prompt_line = encode_line("PROMPT", &prompt);
    stream
        .write_all(prompt_line.as_bytes())
        .context("send prompt to askpass server")?;
    stream.flush().ok();

    let mut response = String::new();
    reader
        .read_line(&mut response)
        .context("read response from askpass server")?;

    let (kind, value) = decode_line(response.trim_end_matches('\n'))?;
    match kind.as_str() {
        "PASSWORD" => {
            print!("{value}");
            std::io::stdout().flush().ok();
            Ok(())
        }
        "CANCEL" => bail!("user cancelled password prompt"),
        "TIMEOUT" => bail!("password prompt timed out"),
        other => bail!("unexpected response kind '{other}'"),
    }
}

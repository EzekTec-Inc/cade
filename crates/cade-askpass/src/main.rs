//! `cade-askpass` — the Askpass helper binary.
//!
//! Invoked by `sudo -A`, `ssh`, or `git` (via `SUDO_ASKPASS` /
//! `SSH_ASKPASS` / `GIT_ASKPASS`) when a password prompt is needed
//! for a process spawned by the CADE bash tool.
//!
//! ## Wire protocol (v1)
//!
//! 1. Read the prompt text from `argv[1]` (the OS passes it to us).
//! 2. Connect to the TCP loopback port specified by
//!    `CADE_ASKPASS_SOCKET=127.0.0.1:<port>`.
//! 3. Send a single line: `PROMPT\t<prompt-text>\n`.  Tabs and
//!    newlines in the prompt are escaped (`\t` → `\\t`, `\n` → `\\n`).
//! 4. Read a single line of the form `PASSWORD\t<value>\n` (same
//!    escaping) or `CANCEL\n`.
//! 5. On `PASSWORD`, write the unescaped value to `stdout` and exit
//!    with `0`.  On `CANCEL` or any error, exit with `1` (the OS
//!    treats this as "user pressed Ctrl-C" and aborts the password
//!    prompt cleanly).
//!
//! ## Security notes
//!
//! - The IPC server in `cade-agent::tools::bash` MUST bind only to
//!   `127.0.0.1` — never to `0.0.0.0` or a Unix socket world-writable
//!   path — and MUST authenticate the client by token (a future
//!   milestone).
//! - The password is held in memory as a `String` for as long as it
//!   takes to write to stdout.  We make no attempt to mlock or
//!   zeroize — that is the responsibility of the calling utility.
//! - We never log the prompt or password.  The only stderr output is
//!   error context with no sensitive content.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use anyhow::{Context, Result, anyhow, bail};

const ENV_SOCKET: &str = "CADE_ASKPASS_SOCKET";
const PROTOCOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn main() {
    if let Err(e) = run() {
        // No prompt or password material in error messages — only the
        // failure mode.  stderr is consumed by the parent utility
        // (sudo, ssh, git) and may be displayed to the user.
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

    let mut stream = TcpStream::connect(&socket)
        .with_context(|| format!("connecting to askpass server at {socket}"))?;
    stream
        .set_read_timeout(Some(PROTOCOL_TIMEOUT))
        .context("set read timeout")?;
    stream
        .set_write_timeout(Some(PROTOCOL_TIMEOUT))
        .context("set write timeout")?;

    let line = encode_line("PROMPT", &prompt);
    stream
        .write_all(line.as_bytes())
        .context("send prompt to askpass server")?;
    stream.flush().ok();

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .context("read response from askpass server")?;

    let (kind, value) = decode_line(response.trim_end_matches('\n'))?;
    match kind.as_str() {
        "PASSWORD" => {
            // Print without trailing newline — sudo/ssh expect just
            // the secret.  The OS adds the newline.
            print!("{value}");
            std::io::stdout().flush().ok();
            Ok(())
        }
        "CANCEL" => bail!("user cancelled password prompt"),
        other => bail!("unexpected response kind '{other}'"),
    }
}

/// Encode one protocol line: `<KIND>\t<value>\n` with escaping.
pub fn encode_line(kind: &str, value: &str) -> String {
    let mut out = String::with_capacity(kind.len() + value.len() + 4);
    out.push_str(kind);
    out.push('\t');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('\n');
    out
}

/// Decode one protocol line.  Returns (kind, value) where the value
/// has been unescaped.  Lines without a tab are kind-only with empty
/// value (e.g. `CANCEL`).
pub fn decode_line(line: &str) -> Result<(String, String)> {
    let (kind, raw_value) = match line.split_once('\t') {
        Some((k, v)) => (k, v),
        None => (line, ""),
    };
    if kind.is_empty() {
        bail!("empty protocol kind");
    }
    let mut value = String::with_capacity(raw_value.len());
    let mut chars = raw_value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => value.push('\\'),
                Some('t') => value.push('\t'),
                Some('n') => value.push('\n'),
                Some(other) => bail!("invalid escape \\{other}"),
                None => bail!("dangling backslash"),
            }
        } else {
            value.push(c);
        }
    }
    Ok((kind.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_simple_prompt_roundtrip() {
        let line = encode_line("PROMPT", "Password for ezektec:");
        assert_eq!(line, "PROMPT\tPassword for ezektec:\n");
        let (k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(k, "PROMPT");
        assert_eq!(v, "Password for ezektec:");
    }

    #[test]
    fn encode_value_with_tab_is_escaped() {
        let line = encode_line("PASSWORD", "hunter2\twith-tab");
        assert!(line.contains("hunter2\\twith-tab"));
        let (k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(k, "PASSWORD");
        assert_eq!(v, "hunter2\twith-tab");
    }

    #[test]
    fn encode_value_with_newline_is_escaped() {
        let line = encode_line("PASSWORD", "line1\nline2");
        // Single line on the wire — exactly one trailing newline.
        assert_eq!(line.matches('\n').count(), 1);
        let (k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(v, "line1\nline2");
    }

    #[test]
    fn encode_value_with_backslash_is_escaped() {
        let line = encode_line("PASSWORD", "back\\slash");
        let (_k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(v, "back\\slash");
    }

    #[test]
    fn cancel_line_has_empty_value() {
        let (k, v) = decode_line("CANCEL").unwrap();
        assert_eq!(k, "CANCEL");
        assert_eq!(v, "");
    }

    #[test]
    fn decode_rejects_empty_kind() {
        let err = decode_line("\tvalue").unwrap_err();
        assert!(err.to_string().contains("empty protocol kind"));
    }

    #[test]
    fn decode_rejects_invalid_escape() {
        let err = decode_line("PASSWORD\tbad\\xescape").unwrap_err();
        assert!(err.to_string().contains("invalid escape"));
    }

    #[test]
    fn decode_rejects_dangling_backslash() {
        let err = decode_line("PASSWORD\thanging\\").unwrap_err();
        assert!(err.to_string().contains("dangling backslash"));
    }

    #[test]
    fn unicode_passwords_round_trip() {
        let line = encode_line("PASSWORD", "пароль🔐ñ");
        let (_k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(v, "пароль🔐ñ");
    }

    #[test]
    fn empty_password_is_legal() {
        let line = encode_line("PASSWORD", "");
        assert_eq!(line, "PASSWORD\t\n");
        let (k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
        assert_eq!(k, "PASSWORD");
        assert_eq!(v, "");
    }
}

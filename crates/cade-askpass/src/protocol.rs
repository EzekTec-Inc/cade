//! Wire-format primitives for the askpass IPC protocol v1.
//!
//! ## Wire protocol (v1)
//!
//! All messages are single newline-terminated lines of the form
//! `<KIND>\t<value>\n` with three escape sequences inside `<value>`:
//!
//! | Raw | Wire |
//! |---|---|
//! | `\\` | `\\\\` |
//! | `\t` | `\\t`  |
//! | `\n` | `\\n`  |
//!
//! ### Sequence
//!
//! ```text
//! client → server : AUTH\t<hex-token>\n
//! server → client : OK\n                  (auth accepted)
//!                 | DENY\n                (auth rejected; server closes)
//! client → server : PROMPT\t<text>\n
//! server → client : PASSWORD\t<value>\n   (user submitted)
//!                 | CANCEL\n              (user pressed Esc)
//!                 | TIMEOUT\n             (server-side deadline)
//! ```
//!
//! On any error or non-`PASSWORD` response, the askpass binary
//! exits non-zero so the calling utility (sudo/ssh) treats it as
//! "user pressed Ctrl-C".

use anyhow::{Result, bail};

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

/// Decode one protocol line.  Returns `(kind, value)` where `value`
/// has been unescaped.  Lines without a tab are kind-only with empty
/// value (e.g. `OK`, `CANCEL`).
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
        assert_eq!(line.matches('\n').count(), 1);
        let (_k, v) = decode_line(line.trim_end_matches('\n')).unwrap();
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
    fn ok_line_has_empty_value() {
        let (k, v) = decode_line("OK").unwrap();
        assert_eq!(k, "OK");
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

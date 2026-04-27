//! Pure SSE frame parser for the cade-gui WASM app.
//!
//! **No browser dependencies.** Parses raw bytes arriving from a
//! `fetch()` + `ReadableStream` (or any other byte source) into typed
//! `SseFrame` values.  The actual network I/O lives in a wasm-only
//! adapter module (M5b); this file is native-testable.
//!
//! # Wire format handled
//!
//! The cade-server SSE streams emit **only** `data:` fields (never
//! `event:`, `id:`, or `retry:`).  Each frame is terminated by a blank
//! line (`\n\n` or `\r\n\r\n`).  Payloads are either:
//!
//! * A single-line JSON object: `{"message_type":"stream_delta",...}`
//! * The literal sentinel: `[DONE]`
//!
//! The parser is future-proofed to silently ignore unknown field names
//! (e.g. `id:`, `retry:`) so the server can add them without breaking
//! the client.
//!
//! # Streaming usage
//!
//! ```ignore
//! let mut parser = SseParser::new();
//! // bytes arrive in arbitrary chunks from the network
//! parser.feed(chunk);
//! while let Some(frame) = parser.pop() {
//!     match frame {
//!         SseFrame::Json(val)            => handle_event(val),
//!         SseFrame::Done                 => break,
//!         SseFrame::ParseError(msg)      => log_warning(msg),
//!     }
//! }
//! ```

use std::collections::VecDeque;

/// A single parsed SSE frame.
#[derive(Debug, Clone, PartialEq)]
pub enum SseFrame {
    /// A successfully decoded JSON payload (any `data:` line whose content
    /// is valid JSON and is not the `[DONE]` sentinel).
    Json(serde_json::Value),
    /// The `[DONE]` sentinel — signals end-of-stream.
    Done,
    /// The `data:` payload was not valid JSON and was not `[DONE]`.
    /// Carries the raw text for diagnostics.
    ParseError(String),
}

/// Incremental SSE frame parser.
///
/// Callers push arbitrary byte slices via [`feed`](Self::feed) and drain
/// complete frames via [`pop`](Self::pop).  The parser handles:
///
/// * Frames split across multiple `feed` calls (byte-by-byte is fine).
/// * Both `\n` and `\r\n` line endings (and mixed within one stream).
/// * Unknown SSE fields (`id:`, `retry:`, etc.) — silently ignored.
/// * Multiple `data:` lines in a single frame — concatenated with `\n`
///   per the SSE specification.
pub struct SseParser {
    /// Bytes not yet consumed into a complete line.
    line_buf: Vec<u8>,
    /// Accumulated `data:` field values for the current frame.
    /// Multiple `data:` lines are joined with `\n` per the SSE spec.
    data_buf: String,
    /// Whether we have seen at least one `data:` field in the current
    /// frame (distinguishes "no data yet" from "data was empty string").
    has_data: bool,
    /// Complete frames ready for the caller to drain.
    pending: VecDeque<SseFrame>,
}

impl SseParser {
    /// Create a fresh parser with empty buffers.
    pub fn new() -> Self {
        Self {
            line_buf: Vec::new(),
            data_buf: String::new(),
            has_data: false,
            pending: VecDeque::new(),
        }
    }

    /// Push a chunk of bytes into the parser.  Zero or more complete
    /// frames may become available via [`pop`](Self::pop) after this call.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' {
                self.process_line();
            } else if b == b'\r' {
                // Swallow \r — we trigger on \n only.  This handles both
                // `\r\n` (the \r is ignored, the \n triggers) and bare
                // `\r` (ignored entirely — matches browser SSE behaviour).
            } else {
                self.line_buf.push(b);
            }
        }
    }

    /// Drain the next complete frame, if any.
    pub fn pop(&mut self) -> Option<SseFrame> {
        self.pending.pop_front()
    }

    /// Called when we encounter `\n`.  An empty line (two consecutive
    /// `\n`s, possibly with `\r` in between) signals the end of a frame.
    fn process_line(&mut self) {
        if self.line_buf.is_empty() {
            // Blank line → dispatch frame if we have accumulated data.
            self.dispatch_frame();
            return;
        }

        let line = String::from_utf8_lossy(&self.line_buf).into_owned();
        self.line_buf.clear();

        // SSE field parsing: `field: value` or `field:value` (space after
        // colon is optional per the spec).
        if let Some(colon) = line.find(':') {
            let field = &line[..colon];
            let mut value = &line[colon + 1..];
            // Per SSE spec: if the first character after the colon is a
            // space, strip it.
            if value.starts_with(' ') {
                value = &value[1..];
            }

            if field == "data" {
                if self.has_data {
                    // Multiple data lines → concatenate with \n.
                    self.data_buf.push('\n');
                }
                self.data_buf.push_str(value);
                self.has_data = true;
            }
            // Unknown fields (id, event, retry, etc.) → ignored.
        }
        // Lines with no colon are comments (starting with ':') or
        // malformed — ignore per spec.
    }

    /// Emit a frame from the accumulated `data:` lines and reset for
    /// the next frame.
    fn dispatch_frame(&mut self) {
        if !self.has_data {
            return;
        }

        let raw = std::mem::take(&mut self.data_buf);
        self.has_data = false;

        let trimmed = raw.trim();
        let frame = if trimmed == "[DONE]" {
            SseFrame::Done
        } else {
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(val) => SseFrame::Json(val),
                Err(_) => SseFrame::ParseError(raw),
            }
        };
        self.pending.push_back(frame);
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── 1. Empty feed → no frames ───────────────────────────────────────

    #[test]
    fn empty_feed_yields_no_frames() {
        let mut p = SseParser::new();
        p.feed(b"");
        assert!(p.pop().is_none());
    }

    // ── 2. Single complete JSON frame ───────────────────────────────────

    #[test]
    fn single_json_frame() {
        let mut p = SseParser::new();
        p.feed(b"data: {\"x\":1}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"x": 1}))));
        assert!(p.pop().is_none());
    }

    // ── 3. [DONE] sentinel ──────────────────────────────────────────────

    #[test]
    fn done_sentinel() {
        let mut p = SseParser::new();
        p.feed(b"data: [DONE]\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Done));
        assert!(p.pop().is_none());
    }

    // ── 4. Two frames in one feed ───────────────────────────────────────

    #[test]
    fn two_frames_in_one_feed() {
        let mut p = SseParser::new();
        p.feed(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"a": 1}))));
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"b": 2}))));
        assert!(p.pop().is_none());
    }

    // ── 5. Frame split across two feeds ─────────────────────────────────

    #[test]
    fn frame_split_across_two_feeds() {
        let mut p = SseParser::new();
        p.feed(b"data: {\"sp");
        assert!(p.pop().is_none(), "no frame yet — incomplete");
        p.feed(b"lit\":true}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"split": true}))));
    }

    // ── 6. Frame split byte-by-byte ─────────────────────────────────────

    #[test]
    fn frame_split_byte_by_byte() {
        let mut p = SseParser::new();
        let input = b"data: {\"k\":\"v\"}\n\n";
        for &b in input {
            p.feed(&[b]);
        }
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"k": "v"}))));
        assert!(p.pop().is_none());
    }

    // ── 7. CRLF line endings ────────────────────────────────────────────

    #[test]
    fn crlf_line_endings() {
        let mut p = SseParser::new();
        p.feed(b"data: {\"cr\":true}\r\n\r\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"cr": true}))));
        assert!(p.pop().is_none());
    }

    // ── 8. Unknown field (id:) ignored, data still parsed ───────────────

    #[test]
    fn unknown_field_ignored() {
        let mut p = SseParser::new();
        p.feed(b"id: 42\ndata: {\"ok\":1}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"ok": 1}))));
        assert!(p.pop().is_none());
    }

    // ── 9. Malformed JSON → ParseError ──────────────────────────────────

    #[test]
    fn malformed_json_yields_parse_error() {
        let mut p = SseParser::new();
        p.feed(b"data: not-json\n\n");
        match p.pop() {
            Some(SseFrame::ParseError(raw)) => {
                assert_eq!(raw, "not-json");
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
        assert!(p.pop().is_none());
    }

    // ── 10. Multiple data: lines concatenated with \n ───────────────────

    #[test]
    fn multiple_data_lines_concatenated() {
        let mut p = SseParser::new();
        // SSE spec: two `data:` lines in one frame → joined by \n.
        // JSON allows whitespace between tokens, so `{"multi":\n"line"}`
        // is actually valid JSON — serde_json parses the \n as whitespace.
        p.feed(b"data: {\"multi\":\ndata: \"line\"}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"multi": "line"}))),);
        assert!(p.pop().is_none());
    }

    // ── 11. Realistic server stream ─────────────────────────────────────

    #[test]
    fn realistic_server_stream() {
        let mut p = SseParser::new();
        // Simulates what cade-server actually sends for a short completion.
        let wire = b"\
data: {\"message_type\":\"stream_start\",\"conversation_id\":\"c1\",\"run_id\":\"r1\"}\n\
\n\
data: {\"message_type\":\"stream_delta\",\"content\":\"fn \"}\n\
\n\
data: {\"message_type\":\"stream_delta\",\"content\":\"main()\"}\n\
\n\
data: {\"message_type\":\"stream_end\"}\n\
\n\
data: [DONE]\n\
\n";
        p.feed(wire);

        assert_eq!(
            p.pop(),
            Some(SseFrame::Json(json!({
                "message_type": "stream_start",
                "conversation_id": "c1",
                "run_id": "r1"
            })))
        );
        assert_eq!(
            p.pop(),
            Some(SseFrame::Json(json!({
                "message_type": "stream_delta",
                "content": "fn "
            })))
        );
        assert_eq!(
            p.pop(),
            Some(SseFrame::Json(json!({
                "message_type": "stream_delta",
                "content": "main()"
            })))
        );
        assert_eq!(
            p.pop(),
            Some(SseFrame::Json(json!({"message_type": "stream_end"})))
        );
        assert_eq!(p.pop(), Some(SseFrame::Done));
        assert!(p.pop().is_none());
    }

    // ── 12. No data: in frame → no frame emitted ────────────────────────

    #[test]
    fn blank_lines_without_data_yield_nothing() {
        let mut p = SseParser::new();
        // Just blank lines, no data: fields — no frames should be emitted.
        p.feed(b"\n\n\n\n");
        assert!(p.pop().is_none());
    }

    // ── 13. data: with no space after colon ─────────────────────────────

    #[test]
    fn data_no_space_after_colon() {
        let mut p = SseParser::new();
        // SSE spec: space after colon is optional.
        p.feed(b"data:{\"tight\":1}\n\n");
        assert_eq!(p.pop(), Some(SseFrame::Json(json!({"tight": 1}))));
    }
}

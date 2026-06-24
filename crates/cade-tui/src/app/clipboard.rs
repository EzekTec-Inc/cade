use super::*;

/// Write `text` to the system clipboard via OSC 52 escape sequence, falling
/// back to `arboard` for native access.  Returns `true` if at least one
/// mechanism succeeded.
pub(crate) fn write_to_clipboard(text: &str) -> bool {
    use base64::Engine;
    use std::io::Write;

    let mut ok = false;

    // 1. OSC 52 universal fallback (works over SSH, tmux, etc.)
    let b64 = base64::prelude::BASE64_STANDARD.encode(text);
    print!("\x1b]52;c;{}\x07", b64);
    let flushed = std::io::stdout().flush().is_ok();
    ok |= flushed;

    // 2. Native OS clipboard (arboard)
    if let Ok(mut cb) = arboard::Clipboard::new() {
        ok |= cb.set_text(text).is_ok();
    }

    ok
}

impl TuiApp {
    #[cfg(not(feature = "clipboard-images"))]
    pub(crate) fn try_paste_image_file_path(&mut self, _text: &str) -> bool {
        false
    }

    #[cfg(feature = "clipboard-images")]
    pub(crate) fn try_paste_image_file_path(&mut self, text: &str) -> bool {
        // Must be a single line — multi-line pastes are never a bare file path.
        if text.contains('\n') {
            return false;
        }

        // Normalise URI → filesystem path.
        let path_str = if let Some(rest) = text.strip_prefix("file://") {
            // `file:///home/…` → `/home/…`  or  `file://localhost/home/…` → `/home/…`
            rest.trim_start_matches("localhost")
                .trim_start_matches('/')
                .to_string()
                .replacen("", "/", 0) // keep as-is; we'll prepend '/' below
        } else {
            text.to_string()
        };

        // Ensure absolute path starts with '/'.
        let path_str = if text.starts_with("file:///") {
            // Strip scheme: file:///absolute/path
            text.trim_start_matches("file://").to_string()
        } else {
            path_str
        };

        // Check extension.
        let ext = std::path::Path::new(&path_str)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let media_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => return false,
        };

        // Read the file and get dimensions.
        let raw = match std::fs::read(&path_str) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let (w, h) = match image::image_dimensions(&path_str) {
            Ok(dims) => dims,
            Err(_) => {
                // Fall back to decoding the bytes to get dimensions.
                match image::load_from_memory(&raw) {
                    Ok(img) => (img.width(), img.height()),
                    Err(_) => return false,
                }
            }
        };

        use base64::Engine;
        let b64 = base64::prelude::BASE64_STANDARD.encode(&raw);
        self.handle_image_paste(media_type, b64, w, h);
        true
    }
}

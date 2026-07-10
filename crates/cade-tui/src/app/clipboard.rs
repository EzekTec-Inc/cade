use super::*;

/// Read raw image pixels from the OS clipboard (arboard) and encode to base64 PNG.
pub(crate) fn read_clipboard_image() -> Option<(String, u32, u32, String)> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;

    let mut png_bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    use image::ImageEncoder;
    if encoder
        .write_image(
            &img.bytes,
            img.width as u32,
            img.height as u32,
            image::ColorType::Rgba8.into(),
        )
        .is_ok()
    {
        use base64::Engine;
        let b64 = base64::prelude::BASE64_STANDARD.encode(&png_bytes);
        Some((
            "image/png".to_string(),
            img.width as u32,
            img.height as u32,
            b64,
        ))
    } else {
        None
    }
}

/// Read text content from the OS clipboard (arboard).
pub(crate) fn read_clipboard_text() -> Option<String> {
    let mut cb = arboard::Clipboard::new().ok()?;
    cb.get_text().ok()
}

impl TuiApp {
    /// Write `text` to the system clipboard via OSC 52 escape sequence, falling
    /// back to `arboard` for native access.  Returns `true` if at least one
    /// mechanism succeeded.
    pub(crate) fn write_to_clipboard(&mut self, text: &str) -> bool {
        use base64::Engine;
        use std::io::Write;

        let mut ok = false;

        // 1. Native OS clipboard (arboard)
        // Headless safety: on Linux, skip arboard if no display server is running
        // to prevent it from throwing stderr warnings/failures that corrupt the Ratatui alternate screen.
        #[cfg(target_os = "linux")]
        let should_try_native =
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
        #[cfg(not(target_os = "linux"))]
        let should_try_native = true;

        if should_try_native {
            if let Some(ref mut cb) = self.clipboard {
                ok = cb.set_text(text).is_ok();
            } else if let Ok(mut cb) = arboard::Clipboard::new() {
                ok = cb.set_text(text).is_ok();
                self.clipboard = Some(cb);
            }
        }

        // 2. Command Line Utilities Fallback (pbcopy, wl-copy, xclip, clip.exe)
        if !ok {
            ok = copy_via_shell_commands(text);
        }

        // 3. OSC 52 universal fallback (with TMUX / Screen passthrough wrapping)
        // Treated as a best-effort, non-blocking side effect so we don't assume writing bytes
        // to stdout guarantees successful OS clipboard synchronization (TUI-Selection Sync Fix).
        let b64 = base64::prelude::BASE64_STANDARD.encode(text);
        let sequence = if std::env::var("TMUX").is_ok() {
            // Tmux passthrough wrapping: escapes raw escape sequences directly to the host terminal emulator
            format!("\x1bPtmux;\x1b\x1b]52;c;{}\x07\x1b\\", b64)
        } else if std::env::var("TERM")
            .map(|t| t.contains("screen"))
            .unwrap_or(false)
        {
            // GNU Screen passthrough wrapping
            format!("\x1bP\x1b]52;c;{}\x07\x1b\\", b64)
        } else {
            // Standard OSC 52 escape sequence
            format!("\x1b]52;c;{}\x07", b64)
        };

        let mut stdout = std::io::stdout().lock();
        if stdout.write_all(sequence.as_bytes()).is_ok() {
            let _ = stdout.flush();
        }

        ok
    }
}

/// Fallback for headless or remote servers: write copied content to ~/.cade/clipboard.txt
#[allow(dead_code)]
pub(crate) fn write_to_file_fallback(text: &str) {
    if let Some(home) = dirs::home_dir() {
        let cade_dir = home.join(".cade");
        if !cade_dir.exists() {
            let _ = std::fs::create_dir_all(&cade_dir);
        }
        let file_path = cade_dir.join("clipboard.txt");
        let _ = std::fs::write(&file_path, text);
    }
}

/// Try to copy text via platform-native command line tools (pbcopy, xclip, wl-copy, clip.exe)
fn copy_via_shell_commands(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // macOS pbcopy
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
    }

    // Linux wl-copy, xclip, xsel, clip.exe
    #[cfg(target_os = "linux")]
    {
        // Try wl-copy (Wayland)
        if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }

        // Try xclip (X11)
        if let Ok(mut child) = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(Stdio::piped())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }

        // Try xsel
        if let Ok(mut child) = Command::new("xsel")
            .arg("--clipboard")
            .arg("--input")
            .stdin(Stdio::piped())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }

        // Try clip.exe (WSL)
        if let Ok(mut child) = Command::new("clip.exe").stdin(Stdio::piped()).spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }
    }

    // Windows native clip
    #[cfg(target_os = "windows")]
    {
        if let Ok(mut child) = Command::new("clip").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
    }

    false
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

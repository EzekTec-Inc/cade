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

/// Read text content from the OS clipboard using native access first, then platform CLI fallbacks.
pub(crate) fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
        .or_else(read_text_via_shell_commands)
}

/// Read text from Linux PRIMARY selection buffer (middle-click buffer).
#[allow(dead_code)]
pub(crate) fn read_linux_primary() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(text) = command_stdout("wl-paste", &["--primary", "--no-newline"]) {
            return Some(text);
        }
        if let Some(text) = command_stdout("xclip", &["-selection", "primary", "-out"]) {
            return Some(text);
        }
        if let Some(text) = command_stdout("xsel", &["--primary", "--output"]) {
            return Some(text);
        }
    }
    None
}

/// Read text from the configured clipboard target buffer.
#[allow(dead_code)]
pub(crate) fn read_clipboard_text_with_mode(
    mode: cade_core::settings::tui::LinuxClipboardSelection,
) -> Option<String> {
    if mode == cade_core::settings::tui::LinuxClipboardSelection::Primary
        && let Some(text) = read_linux_primary()
    {
        return Some(text);
    }
    read_clipboard_text()
}

/// Try to copy text to Linux PRIMARY selection (middle-click buffer).
pub(crate) fn copy_to_linux_primary(text: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        // 1. Wayland PRIMARY (wl-copy --primary)
        if let Ok(mut child) = Command::new("wl-copy")
            .arg("--primary")
            .stdin(Stdio::piped())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }

        // 2. X11 PRIMARY (xclip -selection primary)
        if let Ok(mut child) = Command::new("xclip")
            .arg("-selection")
            .arg("primary")
            .stdin(Stdio::piped())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }

        // 3. X11 PRIMARY (xsel --primary --input)
        if let Ok(mut child) = Command::new("xsel")
            .arg("--primary")
            .arg("--input")
            .stdin(Stdio::piped())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
            && child.wait().map(|s| s.success()).unwrap_or(false)
        {
            return true;
        }
    }

    #[cfg(not(target_os = "linux"))]
    let _ = text;

    false
}

impl TuiApp {
    /// Write `text` to the system clipboard and/or Linux PRIMARY selection based on
    /// `tui_settings.linux_clipboard_selection`.
    ///
    /// Emits OSC 52 directly for instant, non-blocking clipboard synchronization across
    /// tmux and terminal emulators, and offloads native OS tools (arboard, wl-copy, xclip)
    /// to a background worker so the main TUI event loop never hangs.
    pub(crate) fn write_to_clipboard(&mut self, text: &str) -> bool {
        use base64::Engine;
        use std::io::Write;

        let selection_mode = self.tui_settings.linux_clipboard_selection;

        // 1. Instant OSC 52 universal clipboard write (zero latency, no subprocess blocking)
        let b64 = base64::prelude::BASE64_STANDARD.encode(text);
        let sequence = if std::env::var("TMUX").is_ok() {
            // Tmux passthrough wrapping: escapes raw escape sequences directly to the host terminal emulator
            format!("\x1bPtmux;\x1b\x1b]52;c;{}\x07\x1b\\", b64)
        } else if std::env::var("TERM")
            .map(|t| t.contains("screen"))
            .unwrap_or(false)
            || std::env::var("STY").is_ok()
        {
            // GNU Screen passthrough wrapping
            format!("\x1bP\x1b]52;c;{}\x07\x1b\\", b64)
        } else {
            // Standard OSC 52 escape sequence
            format!("\x1b]52;c;{}\x07", b64)
        };

        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(sequence.as_bytes());
        let _ = stdout.flush();

        // 2. Offload native OS clipboard (arboard, wl-copy, xclip) to a non-blocking background thread
        let text_owned = text.to_string();
        std::thread::spawn(move || {
            let _ = copy_to_os_clipboard_bg(&text_owned, selection_mode);
        });

        true
    }
}

fn copy_to_os_clipboard_bg(
    text: &str,
    selection_mode: cade_core::settings::tui::LinuxClipboardSelection,
) -> bool {
    use cade_core::settings::tui::LinuxClipboardSelection;

    let should_write_regular = selection_mode != LinuxClipboardSelection::Primary;
    let should_write_primary = selection_mode != LinuxClipboardSelection::Clipboard;

    let mut regular_ok = false;
    let mut primary_ok = false;

    if should_write_regular {
        #[cfg(target_os = "linux")]
        let should_try_native =
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
        #[cfg(not(target_os = "linux"))]
        let should_try_native = true;

        if should_try_native && let Ok(mut cb) = arboard::Clipboard::new() {
            regular_ok = cb.set_text(text).is_ok();
        }

        if !regular_ok {
            regular_ok = copy_via_shell_commands(text);
        }
    }

    if should_write_primary {
        primary_ok = copy_to_linux_primary(text);
    }

    match selection_mode {
        LinuxClipboardSelection::Clipboard => regular_ok,
        LinuxClipboardSelection::Primary => primary_ok,
        LinuxClipboardSelection::Both => regular_ok || primary_ok,
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
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
        {
            return child.wait().map(|s| s.success()).unwrap_or(false);
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
        if let Ok(mut child) = Command::new("clip").stdin(Stdio::piped()).spawn()
            && let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_ok()
        {
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
    }

    false
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

fn read_text_via_shell_commands() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(text) = command_stdout("pbpaste", &[]) {
            return Some(text);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(text) = command_stdout("wl-paste", &["--no-newline"]) {
            return Some(text);
        }
        if let Some(text) = command_stdout("xclip", &["-selection", "clipboard", "-out"]) {
            return Some(text);
        }
        if let Some(text) = command_stdout("xsel", &["--clipboard", "--output"]) {
            return Some(text);
        }
        if let Some(text) = command_stdout(
            "powershell.exe",
            &["-NoProfile", "-Command", "Get-Clipboard"],
        ) {
            return Some(text);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(text) =
            command_stdout("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])
        {
            return Some(text);
        }
        if let Some(text) = command_stdout(
            "powershell.exe",
            &["-NoProfile", "-Command", "Get-Clipboard"],
        ) {
            return Some(text);
        }
    }

    None
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

    /// Check if the pasted text looks like a `file://` URI or standard file path.
    /// If it points to an existing file, normalize it relative to CWD.
    /// Returns `Some("@path")` if it is inside the project workspace,
    /// `Some("/absolute/path")` if outside, or `None` if it is not a valid file path.
    pub(crate) fn try_normalize_pasted_file_path(&self, text: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains('\n') {
            return None;
        }

        let path_str = if let Some(rest) = trimmed.strip_prefix("file://") {
            rest.trim_start_matches("localhost")
                .trim_start_matches('/')
                .to_string()
        } else {
            trimmed.to_string()
        };

        // Ensure absolute path starts with '/'
        let mut path_buf = std::path::PathBuf::from(&path_str);
        if !path_buf.exists() && !path_str.starts_with('/') {
            let alt_path = format!("/{path_str}");
            let alt_buf = std::path::PathBuf::from(&alt_path);
            if alt_buf.exists() {
                path_buf = alt_buf;
            }
        }

        if path_buf.exists() {
            // Try to make it relative to self.cwd
            let cwd_path = std::path::Path::new(&self.cwd);
            if let Ok(rel) = path_buf.strip_prefix(cwd_path) {
                let rel_str = rel.to_string_lossy().to_string();
                Some(format!("@{}", rel_str))
            } else if let Ok(abs) = path_buf.canonicalize() {
                Some(abs.to_string_lossy().to_string())
            } else {
                Some(path_buf.to_string_lossy().to_string())
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_stdout_returns_none_for_missing_program() {
        assert!(super::command_stdout("cade-command-that-does-not-exist", &[]).is_none());
    }

    #[test]
    fn command_stdout_returns_none_for_failed_program() {
        #[cfg(target_os = "windows")]
        let result = super::command_stdout("cmd", &["/C", "exit 1"]);

        #[cfg(not(target_os = "windows"))]
        let result = super::command_stdout("false", &[]);

        assert!(result.is_none());
    }
}

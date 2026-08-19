#[cfg(feature = "input-control")]
use crate::{Error, Result};

/// Desktop input-control wrapper.
///
/// Gated behind the `input-control` feature (enabled by default) because
/// `enigo` pulls in a non-trivial dependency tree on Linux (x11rb, etc.).
#[cfg(feature = "input-control")]
pub struct DesktopControl {
    enigo: std::sync::Arc<tokio::sync::Mutex<enigo::Enigo>>,
}

#[cfg(feature = "input-control")]
impl DesktopControl {
    /// Try to initialize desktop input control, returning an error if no display server is connected.
    pub async fn try_detect() -> Result<Self> {
        let enigo = enigo::Enigo::new(&enigo::Settings::default())
            .map_err(|e| Error::custom(format!("Failed to initialize desktop input controller: {e}")))?;
        Ok(Self {
            enigo: std::sync::Arc::new(tokio::sync::Mutex::new(enigo)),
        })
    }

    /// Backwards-compatible detector returning Option<Self> (none on headless/no-display environments).
    pub async fn detect() -> Option<Self> {
        Self::try_detect().await.ok()
    }

    pub async fn focus_window(&self, title: &str) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let ps_script = format!(
                "(New-Object -ComObject WScript.Shell).AppActivate('{}')",
                title.replace('\'', "''")
            );
            let status = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_script])
                .status()
                .map_err(|e| Error::custom(format!("Failed to execute PowerShell AppActivate: {e}")))?;

            if status.success() {
                return Ok(());
            }
            return Err(Error::custom(format!("Could not focus window with title '{title}' on Windows")));
        }

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "tell application \"System Events\" to set frontmost of (first process whose name contains \"{}\") to true",
                title.replace('"', "\\\"")
            );
            let status = std::process::Command::new("osascript")
                .args(["-e", &script])
                .status()
                .map_err(|e| Error::custom(format!("Failed to execute osascript: {e}")))?;

            if status.success() {
                return Ok(());
            }
            return Err(Error::custom(format!("Could not focus window with title '{title}' on macOS")));
        }

        #[cfg(target_os = "linux")]
        {
            if let Ok(status) = std::process::Command::new("wmctrl")
                .args(["-a", title])
                .status()
                && status.success()
            {
                return Ok(());
            }

            if let Ok(status) = std::process::Command::new("xdotool")
                .args(["search", "--name", title, "windowactivate"])
                .status()
                && status.success()
            {
                return Ok(());
            }

            Err(Error::custom(format!(
                "Could not focus window with title '{title}' on Linux (wmctrl/xdotool not installed or window not found)"
            )))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Err(Error::custom(format!(
                "Window focusing is unsupported on this operating system. Title: {title}"
            )))
        }
    }

    pub async fn type_text(&self, text: &str) -> Result<()> {
        use enigo::Keyboard;
        let mut enigo = self.enigo.lock().await;
        enigo
            .text(text)
            .map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn key_press(&self, key: &str) -> Result<()> {
        use enigo::{Direction, Key, Keyboard};
        let mut enigo = self.enigo.lock().await;

        let enigo_key = match key.to_lowercase().as_str() {
            "return" | "enter" => Key::Return,
            "escape" | "esc" => Key::Escape,
            "backspace" => Key::Backspace,
            "tab" => Key::Tab,
            "space" => Key::Space,
            "up" => Key::UpArrow,
            "down" => Key::DownArrow,
            "left" => Key::LeftArrow,
            "right" => Key::RightArrow,
            "ctrl" => Key::Control,
            "shift" => Key::Shift,
            "alt" => Key::Alt,
            "meta" | "super" | "win" => Key::Meta,
            k if k.len() == 1 => Key::Unicode(k.chars().next().unwrap()),
            _ => return Err(Error::custom(format!("Unsupported key: {key}"))),
        };

        enigo
            .key(enigo_key, Direction::Click)
            .map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn move_mouse(&self, x: i32, y: i32) -> Result<()> {
        use enigo::{Coordinate, Mouse};
        let mut enigo = self.enigo.lock().await;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn click(&self, button: u8) -> Result<()> {
        use enigo::{Button, Direction, Mouse};
        let mut enigo = self.enigo.lock().await;
        let btn = match button {
            1 => Button::Left,
            2 => Button::Middle,
            3 => Button::Right,
            _ => return Err(Error::custom(format!("Unsupported mouse button: {button}"))),
        };
        enigo
            .button(btn, Direction::Click)
            .map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub fn tool_name(&self) -> &'static str {
        "enigo"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_desktop_control_try_detect_does_not_panic() {
        let _res = DesktopControl::try_detect().await;
    }

    #[tokio::test]
    async fn test_focus_nonexistent_window_returns_error() {
        if let Ok(ctrl) = DesktopControl::try_detect().await {
            let res = ctrl.focus_window("nonexistent_unique_window_title_12345").await;
            assert!(res.is_err(), "Focusing a non-existent window should return an error");
        }
    }
}

//! Unified DesktopCommander Seam (Candidate 2).
//!
//! Provides a single, deep, cross-platform interface for screen observation,
//! window management, mouse/keyboard simulation, and OS notifications.

use async_trait::async_trait;
use crate::{Error, Result};
use super::capture::ScreenCapture;
use super::notify::{send_notification, Urgency};

// region:    --- Types

/// Capture dimensions returned along with base64 PNG data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureResult {
    pub base64_png: String,
    pub width: u32,
    pub height: u32,
}

/// Unified, cross-platform interface for all desktop automation and observation.
#[async_trait]
pub trait DesktopCommander: Send + Sync {
    /// Capture the entire primary display (or indexed monitor).
    async fn capture_screen(&self, monitor_index: Option<usize>) -> Result<CaptureResult>;

    /// Capture a specific application window by title.
    async fn capture_window(&self, window_title: &str) -> Result<CaptureResult>;

    /// List all visible window titles.
    async fn list_windows(&self) -> Result<Vec<String>>;

    /// Focus/activate a window by title.
    async fn focus_window(&self, title: &str) -> Result<()>;

    /// Type text into the currently focused window.
    async fn type_text(&self, text: &str) -> Result<()>;

    /// Press a special key (e.g. "enter", "tab", "escape", "ctrl+c").
    async fn press_key(&self, key: &str) -> Result<()>;

    /// Move mouse cursor to absolute coordinates.
    async fn move_mouse(&self, x: i32, y: i32) -> Result<()>;

    /// Click mouse button ("left", "right", "middle").
    async fn click_mouse(&self, button: &str) -> Result<()>;

    /// Send an OS desktop notification.
    fn notify(&self, title: &str, body: &str, urgency: Urgency) -> Result<()>;
}

// endregion: --- Types

// region:    --- Native Desktop Commander

/// Native adapter implementing DesktopCommander across Linux, macOS, and Windows.
pub struct NativeDesktopCommander {
    capture: ScreenCapture,
    #[cfg(feature = "input-control")]
    control: Option<super::control::DesktopControl>,
}

impl NativeDesktopCommander {
    pub async fn new() -> Self {
        Self {
            capture: ScreenCapture::new(),
            #[cfg(feature = "input-control")]
            control: super::control::DesktopControl::detect().await,
        }
    }
}

#[async_trait]
impl DesktopCommander for NativeDesktopCommander {
    async fn capture_screen(&self, monitor_index: Option<usize>) -> Result<CaptureResult> {
        let (base64_png, width, height) = self.capture.capture_with_dimensions(monitor_index, None).await?;
        Ok(CaptureResult {
            base64_png,
            width,
            height,
        })
    }

    async fn capture_window(&self, window_title: &str) -> Result<CaptureResult> {
        let (base64_png, width, height) = self
            .capture
            .capture_with_dimensions(None, Some(window_title))
            .await?;
        Ok(CaptureResult {
            base64_png,
            width,
            height,
        })
    }

    async fn list_windows(&self) -> Result<Vec<String>> {
        self.capture.list_windows().await
    }

    async fn focus_window(&self, title: &str) -> Result<()> {
        #[cfg(feature = "input-control")]
        if let Some(ref ctrl) = self.control {
            return ctrl.focus_window(title).await;
        }
        Err(Error::custom(format!(
            "Desktop input control is unavailable on this environment to focus '{title}'"
        )))
    }

    async fn type_text(&self, text: &str) -> Result<()> {
        #[cfg(feature = "input-control")]
        if let Some(ref ctrl) = self.control {
            return ctrl.type_text(text).await;
        }
        Err(Error::custom(
            "Desktop input control is unavailable on this environment to type text",
        ))
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        #[cfg(feature = "input-control")]
        if let Some(ref ctrl) = self.control {
            return ctrl.key_press(key).await;
        }
        Err(Error::custom(format!(
            "Desktop input control is unavailable on this environment to press key '{key}'"
        )))
    }

    async fn move_mouse(&self, x: i32, y: i32) -> Result<()> {
        #[cfg(feature = "input-control")]
        if let Some(ref ctrl) = self.control {
            return ctrl.move_mouse(x, y).await;
        }
        Err(Error::custom(
            "Desktop input control is unavailable on this environment to move mouse",
        ))
    }

    async fn click_mouse(&self, button: &str) -> Result<()> {
        #[cfg(feature = "input-control")]
        if let Some(ref ctrl) = self.control {
            let btn_num = match button.to_lowercase().as_str() {
                "left" | "1" => 1,
                "middle" | "2" => 2,
                "right" | "3" => 3,
                _ => 1,
            };
            return ctrl.click(btn_num).await;
        }
        Err(Error::custom(format!(
            "Desktop input control is unavailable on this environment to click '{button}'"
        )))
    }

    fn notify(&self, title: &str, body: &str, urgency: Urgency) -> Result<()> {
        send_notification(title, body, urgency)
    }
}

// endregion: --- Native Desktop Commander

// region:    --- Mock Desktop Commander

/// Mock adapter implementing DesktopCommander for zero-GUI unit testing.
pub struct MockDesktopCommander {
    pub canned_windows: Vec<String>,
    pub canned_capture: CaptureResult,
}

impl Default for MockDesktopCommander {
    fn default() -> Self {
        Self {
            canned_windows: vec!["Terminal".to_string(), "CADE Dashboard".to_string()],
            canned_capture: CaptureResult {
                base64_png: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
                width: 1,
                height: 1,
            },
        }
    }
}

#[async_trait]
impl DesktopCommander for MockDesktopCommander {
    async fn capture_screen(&self, _monitor_index: Option<usize>) -> Result<CaptureResult> {
        Ok(self.canned_capture.clone())
    }

    async fn capture_window(&self, _window_title: &str) -> Result<CaptureResult> {
        Ok(self.canned_capture.clone())
    }

    async fn list_windows(&self) -> Result<Vec<String>> {
        Ok(self.canned_windows.clone())
    }

    async fn focus_window(&self, _title: &str) -> Result<()> {
        Ok(())
    }

    async fn type_text(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    async fn press_key(&self, _key: &str) -> Result<()> {
        Ok(())
    }

    async fn move_mouse(&self, _x: i32, _y: i32) -> Result<()> {
        Ok(())
    }

    async fn click_mouse(&self, _button: &str) -> Result<()> {
        Ok(())
    }

    fn notify(&self, _title: &str, _body: &str, _urgency: Urgency) -> Result<()> {
        Ok(())
    }
}

// endregion: --- Mock Desktop Commander

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_desktop_commander_seam() -> Result<()> {
        let mock = MockDesktopCommander::default();

        let windows = mock.list_windows().await?;
        assert_eq!(windows, vec!["Terminal", "CADE Dashboard"]);

        let capture = mock.capture_screen(None).await?;
        assert_eq!(capture.width, 1);
        assert_eq!(capture.height, 1);

        mock.focus_window("Terminal").await?;
        mock.type_text("cargo test").await?;
        mock.press_key("enter").await?;
        mock.notify("Test", "Body", Urgency::Normal)?;

        Ok(())
    }
}

// endregion: --- Tests
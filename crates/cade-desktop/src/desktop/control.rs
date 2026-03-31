use crate::{Error, Result};

pub struct DesktopControl {
    enigo: std::sync::Arc<tokio::sync::Mutex<enigo::Enigo>>,
}

impl DesktopControl {
    pub async fn detect() -> Self {
        Self {
            enigo: std::sync::Arc::new(tokio::sync::Mutex::new(
                enigo::Enigo::new(&enigo::Settings::default()).expect("Failed to initialize enigo"),
            )),
        }
    }

    pub async fn focus_window(&self, title: &str) -> Result<()> {
        let windows = active_win_pos_rs::get_active_window()
            .map_err(|e| Error::custom(format!("Failed to get active window: {e:?}")))?;
        // `active-win-pos-rs` only gets the active window or lists them, but doesn't focus them directly.
        // For full cross-platform window focusing, we would need to integrate platform-specific code or a different crate.
        // As a placeholder per the plan, we'll log it or return an error.
        Err(Error::custom(format!("Native cross-platform window focusing not fully supported yet. Title: {title}. Active window: {windows:?}")))
    }

    pub async fn type_text(&self, text: &str) -> Result<()> {
        use enigo::Keyboard;
        let mut enigo = self.enigo.lock().await;
        enigo.text(text).map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn key_press(&self, key: &str) -> Result<()> {
        use enigo::{Keyboard, Key, Direction};
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

        enigo.key(enigo_key, Direction::Click).map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn move_mouse(&self, x: i32, y: i32) -> Result<()> {
        use enigo::{Mouse, Coordinate};
        let mut enigo = self.enigo.lock().await;
        enigo.move_mouse(x, y, Coordinate::Abs).map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub async fn click(&self, button: u8) -> Result<()> {
        use enigo::{Mouse, Button, Direction};
        let mut enigo = self.enigo.lock().await;
        let btn = match button {
            1 => Button::Left,
            2 => Button::Middle,
            3 => Button::Right,
            _ => return Err(Error::custom(format!("Unsupported mouse button: {button}"))),
        };
        enigo.button(btn, Direction::Click).map_err(|e| Error::custom(format!("enigo error: {e}")))?;
        Ok(())
    }

    pub fn tool_name(&self) -> &'static str {
        "enigo"
    }
}

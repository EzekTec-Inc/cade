use crate::{Error, Result};
use tokio::process::Command;

/// Window and desktop control via xdotool (X11) or ydotool (Wayland)
pub struct DesktopControl {
    tool: ControlTool,
}

#[derive(Debug, Clone, Copy)]
enum ControlTool {
    Xdotool,
    Ydotool,
}

impl DesktopControl {
    pub async fn detect() -> Self {
        // Prefer xdotool, fall back to ydotool
        let tool = if Self::command_exists("xdotool").await {
            ControlTool::Xdotool
        } else {
            ControlTool::Ydotool
        };
        Self { tool }
    }

    fn new_command(program: &str) -> Command {
        let mut cmd = Command::new(program);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        cmd
    }

    async fn command_exists(cmd: &str) -> bool {
        Self::new_command("which")
            .arg(cmd)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Focus a window by title
    pub async fn focus_window(&self, title: &str) -> Result<()> {
        match self.tool {
            ControlTool::Xdotool => {
                Self::new_command("xdotool")
                    .args(["search", "--name", title, "windowactivate"])
                    .output()
                    .await
                    ?;
            }
            ControlTool::Ydotool => {
                return Err(Error::custom("ydotool does not support window focus by title"));
            }
        }
        Ok(())
    }

    /// Type text into the currently focused window
    pub async fn type_text(&self, text: &str) -> Result<()> {
        match self.tool {
            ControlTool::Xdotool => {
                Self::new_command("xdotool")
                    .args(["type", "--clearmodifiers", text])
                    .output()
                    .await
                    ?;
            }
            ControlTool::Ydotool => {
                Self::new_command("ydotool")
                    .args(["type", text])
                    .output()
                    .await
                    ?;
            }
        }
        Ok(())
    }

    /// Send a key combination (e.g., "ctrl+s", "Return", "Escape")
    pub async fn key_press(&self, key: &str) -> Result<()> {
        match self.tool {
            ControlTool::Xdotool => {
                Self::new_command("xdotool")
                    .args(["key", key])
                    .output()
                    .await
                    ?;
            }
            ControlTool::Ydotool => {
                Self::new_command("ydotool")
                    .args(["key", key])
                    .output()
                    .await
                    ?;
            }
        }
        Ok(())
    }

    /// Move mouse cursor to absolute coordinates
    pub async fn move_mouse(&self, x: i32, y: i32) -> Result<()> {
        match self.tool {
            ControlTool::Xdotool => {
                Self::new_command("xdotool")
                    .args(["mousemove", &x.to_string(), &y.to_string()])
                    .output()
                    .await
                    ?;
            }
            ControlTool::Ydotool => {
                Self::new_command("ydotool")
                    .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                    .output()
                    .await
                    ?;
            }
        }
        Ok(())
    }

    /// Click: button 1=left, 2=middle, 3=right
    pub async fn click(&self, button: u8) -> Result<()> {
        match self.tool {
            ControlTool::Xdotool => {
                Self::new_command("xdotool")
                    .args(["click", &button.to_string()])
                    .output()
                    .await
                    ?;
            }
            ControlTool::Ydotool => {
                Self::new_command("ydotool")
                    .args(["click", &button.to_string()])
                    .output()
                    .await
                    ?;
            }
        }
        Ok(())
    }

    pub fn tool_name(&self) -> &'static str {
        match self.tool {
            ControlTool::Xdotool => "xdotool",
            ControlTool::Ydotool => "ydotool",
        }
    }
}

//! Desktop extension tools — exposed to the Letta agent as callable functions.
//! Wraps src/desktop/* and provides the same dispatch interface as other tools.

use anyhow::Result;
use serde_json::Value;

use cade_desktop::desktop::{
    capture::ScreenCapture,
    control::DesktopControl,
    notify::{Urgency, send_notification},
};

// ── Screen Capture ────────────────────────────────────────────────────────────

pub struct DesktopCaptureTool;

impl DesktopCaptureTool {
    pub async fn run(args: &Value) -> Result<String> {
        use base64::Engine;
        use std::io::Write;

        let monitor     = args["monitor"].as_u64().map(|n| n as usize);
        let window      = args["window_title"].as_str();
        let save_path   = args["save_path"].as_str();

        let capture = ScreenCapture::new();
        let (b64, width, height) = capture.capture_with_dimensions(monitor, window).await?;

        // Decode base64 → PNG bytes
        let png_bytes = base64::prelude::BASE64_STANDARD
            .decode(&b64)
            .map_err(|e| anyhow::anyhow!("base64 decode: {e}"))?;

        // Resolve save path: explicit arg > ~/Pictures/cade_screenshot_<ts>.png > /tmp/
        let dest = if let Some(p) = save_path {
            // Expand ~ manually
            if p.starts_with("~/") {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                home.join(&p[2..])
            } else {
                std::path::PathBuf::from(p)
            }
        } else {
            let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
            let dir  = home.join("Pictures");
            let _    = std::fs::create_dir_all(&dir);
            dir.join(format!("cade_screenshot_{ts}.png"))
        };

        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut f = std::fs::File::create(&dest)
            .map_err(|e| anyhow::anyhow!("Cannot write to {}: {e}", dest.display()))?;
        f.write_all(&png_bytes)?;

        let kb = png_bytes.len() / 1024;
        Ok(format!(
            "Screenshot saved: {} ({}×{} px, {}KB)",
            dest.display(), width, height, kb
        ))
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "desktop_screenshot",
            "description": "Capture a screenshot of the full screen or a specific window and save it to disk. Returns the saved file path and image dimensions. Use save_path to control the destination (supports ~/... expansion).",
            "parameters": {
                "type": "object",
                "properties": {
                    "save_path":    { "type": "string",  "description": "Where to save the PNG, e.g. '~/Pictures/shot.png'. Default: ~/Pictures/cade_screenshot_<timestamp>.png" },
                    "monitor":      { "type": "integer", "description": "Monitor index (0-based, default 0)" },
                    "window_title": { "type": "string",  "description": "Capture a specific window by exact title (optional)" }
                },
                "required": []
            }
        })
    }
}

// ── List Windows ──────────────────────────────────────────────────────────────

pub struct DesktopListWindowsTool;

impl DesktopListWindowsTool {
    pub async fn run(_args: &Value) -> Result<String> {
        let capture = ScreenCapture::new();
        let windows = capture.list_windows().await?;
        if windows.is_empty() {
            Ok("No open windows found".to_string())
        } else {
            Ok(windows.join("\n"))
        }
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "desktop_list_windows",
            "description": "List all visible (non-minimized) window titles on the desktop.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        })
    }
}

// ── Window / App Control ──────────────────────────────────────────────────────

pub struct DesktopControlTool;

impl DesktopControlTool {
    pub async fn run(args: &Value) -> Result<String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("desktop_control: missing 'action'"))?;

        let ctrl = DesktopControl::detect().await;

        match action {
            "focus_window" => {
                let title = args["title"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("focus_window requires 'title'"))?;
                ctrl.focus_window(title).await?;
                Ok(format!("Focused window: {title}"))
            }
            "type_text" => {
                let text = args["text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("type_text requires 'text'"))?;
                ctrl.type_text(text).await?;
                Ok(format!("Typed {} characters", text.len()))
            }
            "key_press" => {
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("key_press requires 'key'"))?;
                ctrl.key_press(key).await?;
                Ok(format!("Pressed key: {key}"))
            }
            "move_mouse" => {
                let x = args["x"]
                    .as_i64()
                    .ok_or_else(|| anyhow::anyhow!("move_mouse requires 'x'"))? as i32;
                let y = args["y"]
                    .as_i64()
                    .ok_or_else(|| anyhow::anyhow!("move_mouse requires 'y'"))? as i32;
                ctrl.move_mouse(x, y).await?;
                Ok(format!("Moved mouse to ({x}, {y})"))
            }
            "click" => {
                let button = args["button"].as_u64().unwrap_or(1) as u8;
                ctrl.click(button).await?;
                Ok(format!("Clicked button {button}"))
            }
            other => Err(anyhow::anyhow!(
                "Unknown action '{other}'. Valid: focus_window, type_text, key_press, move_mouse, click"
            )),
        }
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "desktop_control",
            "description": "Control desktop windows and input. Uses xdotool (X11) or ydotool (Wayland). Actions: focus_window, type_text, key_press, move_mouse, click.",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["focus_window", "type_text", "key_press", "move_mouse", "click"],
                        "description": "Action to perform"
                    },
                    "title":  { "type": "string",  "description": "Window title (for focus_window)" },
                    "text":   { "type": "string",  "description": "Text to type (for type_text)" },
                    "key":    { "type": "string",  "description": "Key combo, e.g. 'ctrl+s', 'Return' (for key_press)" },
                    "x":      { "type": "integer", "description": "X coordinate (for move_mouse)" },
                    "y":      { "type": "integer", "description": "Y coordinate (for move_mouse)" },
                    "button": { "type": "integer", "description": "Mouse button: 1=left, 2=middle, 3=right (for click, default 1)" }
                },
                "required": ["action"]
            }
        })
    }
}

// ── Notifications ─────────────────────────────────────────────────────────────

pub struct DesktopNotifyTool;

impl DesktopNotifyTool {
    pub async fn run(args: &Value) -> Result<String> {
        let title = args["title"].as_str().unwrap_or("CADE");
        let body = args["body"].as_str().unwrap_or("");
        let urgency = match args["urgency"].as_str().unwrap_or("normal") {
            "low" => Urgency::Low,
            "critical" => Urgency::Critical,
            _ => Urgency::Normal,
        };

        send_notification(title, body, urgency)?;
        Ok(format!("Notification sent: [{title}] {body}"))
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "desktop_notify",
            "description": "Send a desktop OS notification to the user. Useful for alerting on task completion, errors, or when user input is needed.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title":   { "type": "string", "description": "Notification title (default: 'CADE')" },
                    "body":    { "type": "string", "description": "Notification body text" },
                    "urgency": {
                        "type": "string",
                        "enum": ["low", "normal", "critical"],
                        "description": "Urgency level (default: 'normal')"
                    }
                },
                "required": ["body"]
            }
        })
    }
}

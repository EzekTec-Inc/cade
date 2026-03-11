use anyhow::{Context, Result};
use base64::Engine;
use std::io::Cursor;

/// Screen capture utilities using xcap (cross-platform, no libpipewire needed)
pub struct ScreenCapture;

impl ScreenCapture {
    pub fn new() -> Self {
        Self
    }

    /// Capture full screen (monitor by index) or a named window.
    /// Returns base64-encoded PNG.
    pub async fn capture(
        &self,
        monitor_index: Option<usize>,
        window_title: Option<&str>,
    ) -> Result<String> {
        let (b64, _, _) = self.capture_with_dimensions(monitor_index, window_title).await?;
        Ok(b64)
    }

    /// Like `capture` but also returns (width, height) of the saved image.
    pub async fn capture_with_dimensions(
        &self,
        monitor_index: Option<usize>,
        window_title: Option<&str>,
    ) -> Result<(String, u32, u32)> {
        let mut image = if let Some(title) = window_title {
            let windows = xcap::Window::all().context("list windows")?;
            let window = windows
                .into_iter()
                .find(|w| w.title().ok().as_deref() == Some(title))
                .with_context(|| format!("no window with title '{title}'"))?;
            window.capture_image().context("capture window")?
        } else {
            let monitors = xcap::Monitor::all().context("list monitors")?;
            let idx = monitor_index.unwrap_or(0);
            let monitor = monitors.get(idx)
                .with_context(|| format!("monitor {idx} not found ({} available)", monitors.len()))?;
            monitor.capture_image().context("capture monitor")?
        };

        // Resize to max 768px wide (reduces token cost for vision models)
        let max_width = 768u32;
        if image.width() > max_width {
            let scale = max_width as f32 / image.width() as f32;
            let new_height = (image.height() as f32 * scale) as u32;
            image = xcap::image::imageops::resize(
                &image,
                max_width,
                new_height,
                xcap::image::imageops::FilterType::Lanczos3,
            );
        }

        let (w, h) = (image.width(), image.height());
        let mut bytes: Vec<u8> = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), xcap::image::ImageFormat::Png)
            .context("encode PNG")?;

        Ok((base64::prelude::BASE64_STANDARD.encode(bytes), w, h))
    }

    /// List all visible window titles
    pub async fn list_windows(&self) -> Result<Vec<String>> {
        let windows = xcap::Window::all().context("list windows")?;
        let titles = windows
            .iter()
            .filter(|w| !w.is_minimized().unwrap_or(true))
            .filter_map(|w| w.title().ok())
            .filter(|t| !t.is_empty() && t != "<No Title>")
            .collect();
        Ok(titles)
    }
}

impl Default for ScreenCapture {
    fn default() -> Self {
        Self::new()
    }
}

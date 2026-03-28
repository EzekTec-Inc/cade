use super::*;

impl TuiApp {
    pub(crate) fn copy_selected_timeline_item_to_clipboard(&mut self) -> bool {
        let Some(text) = self.selected_timeline_item_text() else {
            self.show_toast("No block selected", ToastLevel::Info);
            return false;
        };
        let Ok(mut cb) = arboard::Clipboard::new() else {
            self.show_toast("Clipboard unavailable", ToastLevel::Error);
            return true;
        };
        if cb.set_text(text).is_ok() {
            self.show_toast("Copied selected block", ToastLevel::Success);
        } else {
            self.show_toast("Failed to copy selected block", ToastLevel::Error);
        }
        true
    }


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
        self.editor.handle_image_paste(media_type, b64, w, h);
        true
    }

    /// Try to read an image from the OS clipboard, convert it to a PNG, and
    /// insert a `[image #N: WxH]` placeholder into the editor.
    ///
    /// Returns `true` if an image was found and inserted; `false` otherwise
    /// (caller may fall back to a text paste notification or ignore the event).
    pub(crate) fn try_paste_clipboard_image(&mut self) -> bool {
        // -- Read RGBA data from the clipboard
        let img_data = {
            use arboard::Clipboard;
            let Ok(mut cb) = Clipboard::new() else {
                return false;
            };
            match cb.get_image() {
                Ok(img) => img,
                Err(_) => return false,
            }
        };

        let (w, h) = (img_data.width as u32, img_data.height as u32);
        if w == 0 || h == 0 {
            return false;
        }

        // -- RGBA → PNG → base64
        let b64 = {
            use base64::Engine;
            use image::{ImageBuffer, Rgba};

            // arboard returns raw RGBA bytes; wrap them in an image buffer.
            let owned: Vec<u8> = img_data.bytes.into_owned();
            let Some(rgba) = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, owned) else {
                return false;
            };

            let mut png_buf: Vec<u8> = Vec::new();
            {
                use image::ImageEncoder;
                let enc = image::codecs::png::PngEncoder::new(&mut png_buf);
                if enc
                    .write_image(rgba.as_raw(), w, h, image::ExtendedColorType::Rgba8)
                    .is_err()
                {
                    return false;
                }
            }
            base64::prelude::BASE64_STANDARD.encode(&png_buf)
        };

        // -- Insert into editor
        self.editor.handle_image_paste("image/png", b64, w, h);
        true
    }
}

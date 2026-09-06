//! Terminal Drag-and-Drop and Bracketed Paste Path Sanitizer (`PathDropSanitizer`).
//!
//! Cleans up drag-and-dropped file URIs and escaped path strings from terminal pastes.

// region:    --- Imports

use std::path::{Path, PathBuf};

// endregion: --- Imports

// region:    --- Sanitizer

/// Deep module normalizing dragged and pasted file paths.
pub struct PathDropSanitizer;

impl PathDropSanitizer {
    /// Sanitize a raw pasted string.
    ///
    /// If the string looks like a file path or `file://` URI, normalizes it
    /// and formats it with an `@` reference if it points to a file within `working_dir`.
    pub fn sanitize_paste(raw_input: &str, working_dir: &Path) -> String {
        let trimmed = raw_input.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        // 1. Strip file:// prefix (file:///path on Unix, file://C:/path on Windows)
        let un_uri = if let Some(stripped) = trimmed.strip_prefix("file://") {
            #[cfg(windows)]
            let path_str = stripped.strip_prefix('/').unwrap_or(stripped);
            #[cfg(not(windows))]
            let path_str = stripped;
            path_str
        } else {
            trimmed
        };

        // 2. Decode percent-encoded characters (e.g. %20 -> ' ')
        let percent_decoded = urlencoding::decode(un_uri)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| un_uri.to_string());

        // 3. Remove shell backslash escapes for spaces (e.g. "my\ file.txt" -> "my file.txt")
        let clean_path_str = percent_decoded.replace("\\ ", " ");

        // 4. Resolve path relative to working_dir if possible
        let candidate_path = PathBuf::from(&clean_path_str);
        if candidate_path.exists() {
            if let Ok(rel) = candidate_path.strip_prefix(working_dir) {
                let rel_str = rel.to_string_lossy();
                if rel_str.contains(' ') {
                    return format!("@\"{}\"", rel_str);
                } else {
                    return format!("@{}", rel_str);
                }
            } else {
                let full_str = candidate_path.to_string_lossy();
                if full_str.contains(' ') {
                    return format!("@\"{}\"", full_str);
                } else {
                    return format!("@{}", full_str);
                }
            }
        }

        // If not a standalone existing path, return cleaned string
        clean_path_str
    }
}

// endregion: --- Sanitizer

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_uri_stripping() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_drop.rs");
        let _ = std::fs::write(&test_file, "fn main() {}");

        let uri = format!("file://{}", test_file.to_string_lossy());
        let sanitized = PathDropSanitizer::sanitize_paste(&uri, &temp_dir);
        assert_eq!(sanitized, "@test_drop.rs");

        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn test_percent_encoded_and_escaped_spaces() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test drop with spaces.txt");
        let _ = std::fs::write(&test_file, "hello");

        // Percent encoded URI
        let uri = format!("file://{}", test_file.to_string_lossy().replace(' ', "%20"));
        let sanitized = PathDropSanitizer::sanitize_paste(&uri, &temp_dir);
        assert_eq!(sanitized, "@\"test drop with spaces.txt\"");

        // Shell backslash escaped path
        let escaped = test_file.to_string_lossy().replace(' ', "\\ ");
        let sanitized_esc = PathDropSanitizer::sanitize_paste(&escaped, &temp_dir);
        assert_eq!(sanitized_esc, "@\"test drop with spaces.txt\"");

        let _ = std::fs::remove_file(&test_file);
    }
}

// endregion: --- Tests

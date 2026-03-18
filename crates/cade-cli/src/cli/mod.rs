// region:    --- Modules

pub mod args;
pub mod export_import;
pub mod headless;
pub mod repl;

pub use args::Args;
pub use repl::Repl;

// endregion: --- Modules

/// Truncate a string to `max` *characters* (not bytes), appending "…" if cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

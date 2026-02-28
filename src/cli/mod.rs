pub mod args;
pub mod headless;
pub mod repl;

pub use args::Args;
pub use repl::Repl;

/// Truncate a string to `max` chars, appending "…" if cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

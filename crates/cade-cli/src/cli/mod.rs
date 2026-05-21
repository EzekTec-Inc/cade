// region:    --- Modules

pub mod args;
pub mod eval;
pub mod export_import;
pub mod headless;
pub mod package;
pub mod repl;
pub mod update;

pub use crate::support::text::truncate;
pub use args::{Args, EvalAction, PackageAction, PackageSubcommand};
pub use repl::Repl;

// endregion: --- Modules

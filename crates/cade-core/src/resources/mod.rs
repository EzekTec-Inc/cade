// region:    --- Modules

pub mod context_files;
pub mod packages;
pub mod palette;
pub mod prompts;
pub mod themes;

// endregion: --- Modules

// region:    --- Re-exports

pub use context_files::{ContextFile, ContextScope, build_context_block, discover_context_files};
pub use packages::{PackageManifest, PackageScope, PackageSource, load_manifest, package_root};
pub use prompts::{PromptTemplate, discover_prompts, expand_template};
pub use themes::{
    Theme, ThemeInfo, discover_themes, discover_themes_with_builtins, get_theme,
    list_available_themes, load_theme,
};

// endregion: --- Re-exports

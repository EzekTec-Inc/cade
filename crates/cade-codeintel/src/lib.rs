// region:    --- Modules

mod error;
pub mod index;
pub mod languages;
pub mod query;
pub mod repomap;
pub mod symbol;

// endregion: --- Modules

// region:    --- Re-exports

pub use error::{Error, Result};
pub use index::{Db, ensure_schema, index_repository, update_files};
pub use languages::{Language, SymbolKind, detect_language, get_grammar};
pub use query::{find_references, goto_definition, symbol_search};
pub use repomap::generate_repo_map;
pub use symbol::IndexStats;
pub use symbol::{Symbol, SymbolRef};

// endregion: --- Re-exports

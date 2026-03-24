/// Symbol types for the code-intelligence index.
use serde::{Deserialize, Serialize};

// region:    --- Symbol

/// A single symbol extracted from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id:          String,
    pub repo_root:   String,
    pub file_path:   String,   // relative to repo_root
    pub name:        String,
    pub kind:        String,   // from SymbolKind::as_str()
    pub language:    String,
    pub line_start:  u32,
    pub line_end:    u32,
    pub parent_name: Option<String>,
    pub signature:   Option<String>,
    pub doc_comment: Option<String>,
    pub indexed_at:  i64,
}

/// Statistics from an indexing run.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub files_indexed:  usize,
    pub files_skipped:  usize,
    pub symbols_added:  usize,
    pub symbols_removed: usize,
    pub duration_ms:    u128,
}

// endregion: --- Symbol

// region:    --- SymbolRef (cross-reference)

/// A reference to a symbol at a specific location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRef {
    pub file_path:  String,
    pub line:       u32,
    pub column:     u32,
    pub context:    String,  // surrounding line of code
}

// endregion: --- SymbolRef

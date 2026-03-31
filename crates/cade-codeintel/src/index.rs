/// Repository indexer: walks the source tree, parses files with tree-sitter,
/// extracts symbols, and stores them in SQLite.
///
/// The indexer is designed to be run:
/// - Once on agent creation (full index)
/// - Incrementally when files change (hash-based skip)
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rusqlite::{Connection, params};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::Result;
use crate::languages::{Language, SymbolKind, detect_language, get_grammar};
use crate::symbol::{IndexStats, Symbol};

// -- Type alias for the shared DB handle (mirrors cade-server)
pub type Db = Arc<Mutex<Connection>>;

// region:    --- Schema

/// Ensure the code-intelligence tables exist in the database.
/// Safe to call multiple times (idempotent).
pub fn ensure_schema(db: &Db) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS symbols (
            id           TEXT PRIMARY KEY,
            repo_root    TEXT NOT NULL,
            file_path    TEXT NOT NULL,
            name         TEXT NOT NULL,
            kind         TEXT NOT NULL,
            language     TEXT NOT NULL,
            line_start   INTEGER NOT NULL,
            line_end     INTEGER NOT NULL,
            parent_name  TEXT,
            signature    TEXT,
            doc_comment  TEXT,
            indexed_at   INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_symbols_name  ON symbols(name COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_symbols_file  ON symbols(file_path);
        CREATE INDEX IF NOT EXISTS idx_symbols_repo  ON symbols(repo_root);
        CREATE INDEX IF NOT EXISTS idx_symbols_kind  ON symbols(kind);

        CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
            name, doc_comment, signature,
            content='symbols', content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
            INSERT INTO symbols_fts(rowid, name, doc_comment, signature)
            VALUES (new.rowid, new.name, new.doc_comment, new.signature);
        END;
        CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
            INSERT INTO symbols_fts(symbols_fts, rowid, name, doc_comment, signature)
            VALUES ('delete', old.rowid, old.name, old.doc_comment, old.signature);
        END;

        CREATE TABLE IF NOT EXISTS symbol_index_files (
            repo_root  TEXT NOT NULL,
            file_path  TEXT NOT NULL,
            file_hash  TEXT NOT NULL,
            indexed_at INTEGER NOT NULL,
            PRIMARY KEY (repo_root, file_path)
        );
    "#,
    )?;
    Ok(())
}

// endregion: --- Schema

// region:    --- Indexer

/// Build or refresh the symbol index for a repository.
pub async fn index_repository(repo_root: &Path, db: &Db) -> Result<IndexStats> {
    ensure_schema(db)?;
    let t0 = Instant::now();
    let mut stats = IndexStats::default();
    let repo_root_str = repo_root.to_string_lossy().to_string();

    let walker = WalkDir::new(repo_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "target" | "node_modules" | ".git" | ".svn" | "vendor" | "__pycache__"
            )
        });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let lang = detect_language(path);
        if lang == Language::Unknown {
            continue;
        }

        let rel_path = path
            .strip_prefix(repo_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string());

        let Ok(content) = std::fs::read_to_string(path) else {
            stats.files_skipped += 1;
            continue;
        };

        // Hash-based skip: only re-index if file changed
        let hash = compute_hash(&content);
        if !needs_reindex(db, &repo_root_str, &rel_path, &hash) {
            stats.files_skipped += 1;
            continue;
        }

        // Parse and extract symbols
        let symbols = extract_symbols(&content, lang, &repo_root_str, &rel_path);
        let symbol_count = symbols.len();

        // Remove old symbols for this file, insert new ones
        replace_file_symbols(db, &repo_root_str, &rel_path, symbols)?;
        record_file_hash(db, &repo_root_str, &rel_path, &hash)?;

        stats.files_indexed += 1;
        stats.symbols_added += symbol_count;
    }

    stats.duration_ms = t0.elapsed().as_millis();
    tracing::info!(
        "Indexed {} files ({} symbols) in {}ms",
        stats.files_indexed,
        stats.symbols_added,
        stats.duration_ms
    );
    Ok(stats)
}

/// Incrementally re-index a list of changed files.
pub async fn update_files(changed: &[PathBuf], repo_root: &Path, db: &Db) -> Result<()> {
    ensure_schema(db)?;
    let repo_root_str = repo_root.to_string_lossy().to_string();

    for path in changed {
        let lang = detect_language(path);
        if lang == Language::Unknown {
            continue;
        }

        let rel_path = path
            .strip_prefix(repo_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string());

        if path.exists() {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let hash = compute_hash(&content);
            let symbols = extract_symbols(&content, lang, &repo_root_str, &rel_path);
            replace_file_symbols(db, &repo_root_str, &rel_path, symbols)?;
            record_file_hash(db, &repo_root_str, &rel_path, &hash)?;
        } else {
            // File deleted
            remove_file_symbols(db, &repo_root_str, &rel_path)?;
        }
    }
    Ok(())
}

// endregion: --- Indexer

// region:    --- Symbol extraction

fn extract_symbols(content: &str, lang: Language, repo_root: &str, rel_path: &str) -> Vec<Symbol> {
    let Some(grammar) = get_grammar(lang) else {
        return vec![];
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return vec![];
    }

    let Some(tree) = parser.parse(content, None) else {
        return vec![];
    };
    let root = tree.root_node();
    let source_bytes = content.as_bytes();
    let now = chrono::Utc::now().timestamp();

    extract_from_node(&root, source_bytes, lang, repo_root, rel_path, None, now)
}

fn extract_from_node(
    node: &tree_sitter::Node,
    source: &[u8],
    lang: Language,
    repo_root: &str,
    rel_path: &str,
    parent_name: Option<&str>,
    now: i64,
) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let kind_opt = node_to_symbol_kind(node.kind(), lang);

    if let Some(kind) = kind_opt
        && let Some(name) = extract_name(node, source)
    {
        let start = node.start_position().row as u32 + 1;
        let end = node.end_position().row as u32 + 1;
        let doc = extract_doc_comment(node, source);
        let sig = extract_signature(node, source);

        symbols.push(Symbol {
            id: format!("sym-{}", Uuid::new_v4()),
            repo_root: repo_root.to_string(),
            file_path: rel_path.to_string(),
            name: name.clone(),
            kind: kind.as_str().to_string(),
            language: lang.as_str().to_string(),
            line_start: start,
            line_end: end,
            parent_name: parent_name.map(String::from),
            signature: sig,
            doc_comment: doc,
            indexed_at: now,
        });

        // Recurse into children with this symbol as parent
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            symbols.extend(extract_from_node(
                &child,
                source,
                lang,
                repo_root,
                rel_path,
                Some(&name),
                now,
            ));
        }
        return symbols;
    }

    // No symbol at this node — recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        symbols.extend(extract_from_node(
            &child,
            source,
            lang,
            repo_root,
            rel_path,
            parent_name,
            now,
        ));
    }
    symbols
}

fn node_to_symbol_kind(ts_kind: &str, lang: Language) -> Option<SymbolKind> {
    match (ts_kind, lang) {
        // Rust
        ("function_item", Language::Rust) => Some(SymbolKind::Function),
        ("impl_item", Language::Rust) => None, // too noisy
        ("struct_item", Language::Rust) => Some(SymbolKind::Struct),
        ("enum_item", Language::Rust) => Some(SymbolKind::Enum),
        ("trait_item", Language::Rust) => Some(SymbolKind::Trait),
        ("type_item", Language::Rust) => Some(SymbolKind::Type),
        ("const_item", Language::Rust) => Some(SymbolKind::Const),
        ("mod_item", Language::Rust) => Some(SymbolKind::Module),
        // Python
        ("function_definition", Language::Python) => Some(SymbolKind::Function),
        ("class_definition", Language::Python) => Some(SymbolKind::Class),
        // TypeScript / JavaScript
        ("function_declaration", _) => Some(SymbolKind::Function),
        ("class_declaration", _) => Some(SymbolKind::Class),
        ("interface_declaration", _) => Some(SymbolKind::Interface),
        ("type_alias_declaration", _) => Some(SymbolKind::Type),
        ("method_definition", _) => Some(SymbolKind::Method),
        // Go
        ("method_declaration", Language::Go) => Some(SymbolKind::Method),
        ("type_declaration", Language::Go) => Some(SymbolKind::Type),
        _ => None,
    }
}

fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "name" || child.kind() == "identifier" {
            let text = child.utf8_text(source).ok()?;
            return Some(text.to_string());
        }
    }
    None
}

fn extract_doc_comment(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look at the preceding sibling for doc comments
    let prev = node.prev_named_sibling()?;
    if prev.kind().contains("comment") || prev.kind().contains("doc") {
        let text = prev.utf8_text(source).ok()?;
        return Some(
            text.trim_start_matches('/')
                .trim_start_matches('*')
                .trim()
                .to_string(),
        );
    }
    None
}

fn extract_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Return the first line of the node as a rough signature
    let text = node.utf8_text(source).ok()?;
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim();
    if trimmed.len() > 120 {
        Some(format!("{}…", &trimmed[..120]))
    } else if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// endregion: --- Symbol extraction

// region:    --- DB helpers

fn needs_reindex(db: &Db, repo_root: &str, file_path: &str, new_hash: &str) -> bool {
    let Ok(conn) = db.lock() else { return true; };
    let stored: Option<String> = conn
        .query_row(
            "SELECT file_hash FROM symbol_index_files WHERE repo_root = ?1 AND file_path = ?2",
            params![repo_root, file_path],
            |r| r.get(0),
        )
        .ok();
    stored.as_deref() != Some(new_hash)
}

fn replace_file_symbols(
    db: &Db,
    repo_root: &str,
    file_path: &str,
    symbols: Vec<Symbol>,
) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "DELETE FROM symbols WHERE repo_root = ?1 AND file_path = ?2",
        params![repo_root, file_path],
    )?;
    for sym in &symbols {
        conn.execute(
            "INSERT INTO symbols (id, repo_root, file_path, name, kind, language, line_start, line_end, parent_name, signature, doc_comment, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                sym.id, sym.repo_root, sym.file_path, sym.name, sym.kind,
                sym.language, sym.line_start, sym.line_end, sym.parent_name,
                sym.signature, sym.doc_comment, sym.indexed_at
            ],
        )?;
    }
    Ok(())
}

fn remove_file_symbols(db: &Db, repo_root: &str, file_path: &str) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "DELETE FROM symbols WHERE repo_root = ?1 AND file_path = ?2",
        params![repo_root, file_path],
    )?;
    conn.execute(
        "DELETE FROM symbol_index_files WHERE repo_root = ?1 AND file_path = ?2",
        params![repo_root, file_path],
    )?;
    Ok(())
}

fn record_file_hash(db: &Db, repo_root: &str, file_path: &str, hash: &str) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "INSERT OR REPLACE INTO symbol_index_files (repo_root, file_path, file_hash, indexed_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![repo_root, file_path, hash, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

fn compute_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

// endregion: --- DB helpers

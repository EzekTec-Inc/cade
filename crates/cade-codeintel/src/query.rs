/// Symbol query functions: search, references, definition.
use rusqlite::params;

use crate::Result;
use crate::index::Db;
use crate::symbol::{Symbol, SymbolRef};

// region:    --- Symbol search

/// Full-text search over symbol names and doc comments.
pub fn symbol_search(db: &Db, query: &str, limit: usize) -> Result<Vec<Symbol>> {
    let conn = db.lock().expect("db lock poisoned");

    // Try FTS5 first
    let fts_result = conn
        .prepare(
            "SELECT s.id, s.repo_root, s.file_path, s.name, s.kind, s.language,
                s.line_start, s.line_end, s.parent_name, s.signature, s.doc_comment, s.indexed_at
         FROM symbols s
         JOIN symbols_fts fts ON fts.rowid = s.rowid
         WHERE symbols_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![query, limit as i64], map_symbol_row)
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        });

    match fts_result {
        Ok(rows) if !rows.is_empty() => return Ok(rows),
        _ => {}
    }

    // Fallback: LIKE search
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, repo_root, file_path, name, kind, language,
                line_start, line_end, parent_name, signature, doc_comment, indexed_at
         FROM symbols
         WHERE name LIKE ?1 OR doc_comment LIKE ?1
         ORDER BY name
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![pattern, limit as i64], map_symbol_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

// endregion: --- Symbol search

// region:    --- Definition lookup

/// Find the definition of a symbol by exact name.
/// When `from_file` is provided, symbols in the same file are preferred.
pub fn goto_definition(db: &Db, name: &str, from_file: Option<&str>) -> Result<Option<Symbol>> {
    let conn = db.lock().expect("db lock poisoned");

    // Prefer same-file definition
    if let Some(file) = from_file {
        let result = conn
            .query_row(
                "SELECT id, repo_root, file_path, name, kind, language,
                    line_start, line_end, parent_name, signature, doc_comment, indexed_at
             FROM symbols
             WHERE name = ?1 AND file_path = ?2
             LIMIT 1",
                params![name, file],
                map_symbol_row,
            )
            .ok();
        if result.is_some() {
            return Ok(result);
        }
    }

    // Global lookup
    let result = conn
        .query_row(
            "SELECT id, repo_root, file_path, name, kind, language,
                line_start, line_end, parent_name, signature, doc_comment, indexed_at
         FROM symbols
         WHERE name = ?1
         ORDER BY kind  -- prefer function/struct over module
         LIMIT 1",
            params![name],
            map_symbol_row,
        )
        .ok();
    Ok(result)
}

// endregion: --- Definition lookup

// region:    --- References

/// Find all references to a symbol by searching for its name in source files.
/// This is a text-based approximation; for precise results, use LSP.
pub fn find_references(db: &Db, name: &str, repo_root: &str) -> Result<Vec<SymbolRef>> {
    // Get all files in the repo from the index — drop lock before filesystem I/O
    let file_paths: Vec<String> = {
        let conn = db.lock().expect("db lock poisoned");
        let mut stmt =
            conn.prepare("SELECT DISTINCT file_path FROM symbols WHERE repo_root = ?1 LIMIT 500")?;
        stmt.query_map(params![repo_root], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    }; // lock released here

    let mut refs = Vec::new();
    let pattern = regex::Regex::new(&format!(r"\b{}\b", regex::escape(name)));
    let Ok(re) = pattern else { return Ok(vec![]) };

    for rel_path in &file_paths {
        let abs_path = std::path::Path::new(repo_root).join(rel_path);
        let Ok(content) = std::fs::read_to_string(&abs_path) else {
            continue;
        };

        for (line_idx, line) in content.lines().enumerate() {
            for m in re.find_iter(line) {
                refs.push(SymbolRef {
                    file_path: rel_path.clone(),
                    line: (line_idx + 1) as u32,
                    column: m.start() as u32,
                    context: line.trim().to_string(),
                });
            }
        }
    }

    // Sort by file then line
    refs.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    Ok(refs)
}

// endregion: --- References

// region:    --- Support

fn map_symbol_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    Ok(Symbol {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        file_path: row.get(2)?,
        name: row.get(3)?,
        kind: row.get(4)?,
        language: row.get(5)?,
        line_start: row.get(6)?,
        line_end: row.get(7)?,
        parent_name: row.get(8)?,
        signature: row.get(9)?,
        doc_comment: row.get(10)?,
        indexed_at: row.get(11)?,
    })
}

// endregion: --- Support

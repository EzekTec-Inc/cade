/// Repository map generation.
///
/// Produces a compact text representation of the codebase:
///   - Top N symbols per file (kind, name, line)
///
/// Output is intentionally compact to minimise token usage.
use std::collections::HashMap;
use std::path::Path;

use crate::Result;
use crate::index::Db;

// region:    --- Repo map

/// Generate a compact repo map.
pub fn generate_repo_map(repo_root: &Path, db: &Db, max_symbols_per_file: usize) -> Result<String> {
    let repo_root_str = repo_root.to_string_lossy().to_string();
    let conn = db
        .lock()
        .map_err(|e| crate::Error::custom(format!("db lock poisoned: {e}")))?;

    let mut stmt = conn.prepare(
        "SELECT file_path, name, kind, line_start, parent_name, signature
         FROM symbols
         WHERE repo_root = ?1
         ORDER BY file_path, line_start",
    )?;

    // (file_path, [(name, kind, line_start, parent_name, signature)])
    #[allow(clippy::type_complexity)]
    let mut by_file: HashMap<
        String,
        Vec<(String, String, u32, Option<String>, Option<String>)>,
    > = HashMap::new();
    let mut files_ordered: Vec<String> = Vec::new();

    let rows = stmt.query_map(rusqlite::params![repo_root_str], |row| {
        Ok((
            row.get::<_, String>(0)?,         // file_path
            row.get::<_, String>(1)?,         // name
            row.get::<_, String>(2)?,         // kind
            row.get::<_, u32>(3)?,            // line_start
            row.get::<_, Option<String>>(4)?, // parent_name
            row.get::<_, Option<String>>(5)?, // signature
        ))
    })?;

    for row in rows.filter_map(|r| r.ok()) {
        let (file_path, name, kind, line, parent, sig) = row;
        if !by_file.contains_key(&file_path) {
            files_ordered.push(file_path.clone());
        }
        by_file
            .entry(file_path)
            .or_default()
            .push((name, kind, line, parent, sig));
    }
    drop(stmt);
    drop(conn); // release lock before building string

    files_ordered.sort();

    let mut out = String::from("# Repository Map\n\n");

    for file_path in &files_ordered {
        let entries = match by_file.get(file_path) {
            Some(e) if !e.is_empty() => e,
            _ => continue,
        };

        out.push_str(&format!("## {file_path}\n"));

        let limit = max_symbols_per_file.min(entries.len());
        for (name, kind, line, parent, sig) in entries.iter().take(limit) {
            let parent_str = parent
                .as_deref()
                .map(|p| format!("{p}::"))
                .unwrap_or_default();
            let sig_str = sig.as_deref().unwrap_or(name.as_str());
            out.push_str(&format!(
                "  {kind} {parent_str}{name}  (line {line})  {sig_str}\n"
            ));
        }

        if entries.len() > limit {
            out.push_str(&format!("  … {} more symbols\n", entries.len() - limit));
        }
        out.push('\n');
    }

    if files_ordered.is_empty() {
        out.push_str("(No symbols indexed yet. Run `index_repository` first.)\n");
    }

    Ok(out)
}

// endregion: --- Repo map

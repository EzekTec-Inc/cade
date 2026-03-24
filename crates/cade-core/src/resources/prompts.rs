/// Prompt template discovery and expansion.
///
/// Templates are Markdown files that expand when the user types `/name`.
/// They are discovered from:
///   - `~/.cade/prompts/*.md`        (global)
///   - `.cade/prompts/*.md`          (project)
///   - Any extra paths in settings
///
/// Frontmatter (`---` block) supports a `description` key.
use std::path::{Path, PathBuf};

// region:    --- Types

#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// Command name without the leading `/` (derived from filename stem).
    pub name: String,
    /// One-line description shown in autocomplete.
    pub description: String,
    /// Full template content (frontmatter stripped).
    pub content: String,
    /// Source file path.
    pub source: PathBuf,
}

// endregion: --- Types

// region:    --- Discovery

/// Discover prompt templates from the standard locations.
///
/// Returns templates sorted by name.  Project-local templates shadow global
/// ones with the same name.
pub fn discover_prompts(cwd: &Path, agent_dir: &Path) -> Vec<PromptTemplate> {
    let mut templates: Vec<PromptTemplate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // -- Project-local (higher priority — loaded first so shadowing works)
    let project_dir = cwd.join(".cade").join("prompts");
    load_from_dir(&project_dir, &mut templates, &mut seen);

    // -- Global
    let global_dir = agent_dir.join("prompts");
    load_from_dir(&global_dir, &mut templates, &mut seen);

    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

// endregion: --- Discovery

// region:    --- Expansion

/// Expand a template with positional arguments.
///
/// Substitutions supported:
/// - `$1`, `$2`, ... — individual positional args
/// - `$@` or `$ARGUMENTS` — all args joined by space
/// - `${@:N}` — all args from the N-th (1-indexed)
/// - `${@:N:L}` — L args starting at position N
pub fn expand_template(content: &str, args_str: &str) -> String {
    let args: Vec<&str> = if args_str.trim().is_empty() {
        vec![]
    } else {
        args_str.split_whitespace().collect()
    };

    let mut result = content.to_string();

    // $@ and $ARGUMENTS → all args
    let all_args = args.join(" ");
    result = result.replace("$@", &all_args);
    result = result.replace("$ARGUMENTS", &all_args);

    // ${@:N:L} — slice
    result = replace_slice_refs(&result, &args);

    // ${@:N} — from N onwards
    result = replace_from_refs(&result, &args);

    // $1, $2, ... — positional
    for (i, arg) in args.iter().enumerate() {
        result = result.replace(&format!("${}", i + 1), arg);
    }

    result
}

// endregion: --- Expansion

// region:    --- Support

fn load_from_dir(
    dir: &Path,
    templates: &mut Vec<PromptTemplate>,
    seen: &mut std::collections::HashSet<String>,
) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        // Project shadows global — skip if we already have it
        if seen.contains(&name) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else { continue };
        let (description, content) = parse_prompt_file(&raw);
        seen.insert(name.clone());
        templates.push(PromptTemplate { name, description, content, source: path });
    }
}

/// Parse a prompt template file.
/// Returns `(description, content_without_frontmatter)`.
fn parse_prompt_file(raw: &str) -> (String, String) {
    let trimmed = raw.trim_start();
    if let Some(after) = trimmed.strip_prefix("---")
        && let Some(end) = after.find("---")
    {
        let fm = &after[..end];
        let body = &after[end + 3..];
        let description = extract_fm_field(fm, "description")
            .unwrap_or_else(|| first_nonempty_line(body));
        return (description, body.trim_start().to_string());
    }
    // No frontmatter — use first non-empty line as description
    let description = first_nonempty_line(raw);
    (description, raw.to_string())
}

fn extract_fm_field(fm: &str, key: &str) -> Option<String> {
    fm.lines()
        .find(|l| l.trim_start().starts_with(&format!("{key}:")))
        .map(|l| l.split_once(':').map(|x| x.1).unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

fn first_nonempty_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim()
        .to_string()
}

fn replace_slice_refs(s: &str, args: &[&str]) -> String {
    let mut result = s.to_string();
    // Pattern: ${@:N:L}
    while let Some(start) = result.find("${@:") {
        let rest = &result[start + 4..];
        let Some(end) = rest.find('}') else { break };
        let inner = &rest[..end];
        let parts: Vec<&str> = inner.splitn(2, ':').collect();
        let replacement = if parts.len() == 2 {
            let n: usize = parts[0].parse::<usize>().unwrap_or(1).saturating_sub(1);
            let l: usize = parts[1].parse().unwrap_or(0);
            args.iter().skip(n).take(l).copied().collect::<Vec<_>>().join(" ")
        } else {
            String::new()
        };
        let full_match = format!("${{@:{}}}", inner);
        result = result.replacen(&full_match, &replacement, 1);
    }
    result
}

fn replace_from_refs(s: &str, args: &[&str]) -> String {
    let mut result = s.to_string();
    // Pattern: ${@:N} (no second colon)
    while let Some(start) = result.find("${@:") {
        let rest = &result[start + 4..];
        let Some(end) = rest.find('}') else { break };
        let inner = &rest[..end];
        if inner.contains(':') {
            break; // handled by slice_refs
        }
        let n: usize = inner.parse::<usize>().unwrap_or(1).saturating_sub(1);
        let replacement = args.iter().skip(n).copied().collect::<Vec<_>>().join(" ");
        let full_match = format!("${{@:{}}}", inner);
        result = result.replacen(&full_match, &replacement, 1);
    }
    result
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_dir() -> TempDir { tempfile::tempdir().expect("tempdir") }

    #[test]
    fn test_discover_prompts_empty() {
        // -- Setup & Fixtures
        let cwd = make_dir();
        let agent_dir = make_dir();

        // -- Exec
        let prompts = discover_prompts(cwd.path(), agent_dir.path());

        // -- Check
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_discover_prompts_global() {
        // -- Setup & Fixtures
        let cwd = make_dir();
        let agent_dir = make_dir();
        let prompts_dir = agent_dir.path().join("prompts");
        fs::create_dir_all(&prompts_dir).unwrap();
        fs::write(prompts_dir.join("review.md"), "Review this code for bugs.").unwrap();

        // -- Exec
        let prompts = discover_prompts(cwd.path(), agent_dir.path());

        // -- Check
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "review");
        assert!(prompts[0].content.contains("Review this code"));
    }

    #[test]
    fn test_discover_prompts_project_shadows_global() {
        // -- Setup & Fixtures
        let cwd = make_dir();
        let agent_dir = make_dir();
        let global_dir = agent_dir.path().join("prompts");
        let project_dir = cwd.path().join(".cade").join("prompts");
        fs::create_dir_all(&global_dir).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(global_dir.join("review.md"), "Global review.").unwrap();
        fs::write(project_dir.join("review.md"), "Project review.").unwrap();

        // -- Exec
        let prompts = discover_prompts(cwd.path(), agent_dir.path());

        // -- Check — only one "review", from project
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].content.contains("Project review"));
    }

    #[test]
    fn test_expand_template_positional() {
        // -- Exec
        let result = expand_template("Hello $1, you are $2!", "Alice Engineer");

        // -- Check
        assert_eq!(result, "Hello Alice, you are Engineer!");
    }

    #[test]
    fn test_expand_template_all_args() {
        // -- Exec
        let result = expand_template("Focus on: $@", "performance security");

        // -- Check
        assert_eq!(result, "Focus on: performance security");
    }

    #[test]
    fn test_expand_template_no_args() {
        // -- Exec
        let result = expand_template("Static template with $@", "");

        // -- Check
        assert_eq!(result, "Static template with ");
    }

    #[test]
    fn test_parse_prompt_file_with_frontmatter() {
        // -- Setup & Fixtures
        let raw = "---\ndescription: Review staged changes\n---\nReview the staged diff.";

        // -- Exec
        let (desc, content) = parse_prompt_file(raw);

        // -- Check
        assert_eq!(desc, "Review staged changes");
        assert!(content.contains("staged diff"));
    }
}

// endregion: --- Tests

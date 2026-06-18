// region:    --- Modules

use std::path::{Path, PathBuf};

// endregion: --- Modules

#[cfg(feature = "http")]
use super::parsing::*;
#[cfg(feature = "http")]
use crate::{Error, Result};

use super::types::*;
// -- Live file watcher

/// Spawn a background thread that watches all skills directories for changes.
/// Any Create / Modify / Remove event sends a `()` on the returned channel.
/// The REPL polls the receiver each loop iteration to trigger a reload.
///
/// Uses `notify` 6.x `RecommendedWatcher` (inotify on Linux, FSEvents on macOS,
/// ReadDirectoryChangesW on Windows). The watcher runs on a dedicated std thread
/// (notify is not async-native) and forwards events to a `tokio::sync::mpsc` channel.
pub fn spawn_skill_watcher(cwd: &Path) -> tokio::sync::mpsc::Receiver<()> {
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    let (tx, rx) = tokio::sync::mpsc::channel::<()>(8);

    // Collect directories to watch
    let home = dirs::home_dir();
    let cade_home = home.as_ref().map(|h| h.join(".cade"));

    let mut watch_dirs: Vec<PathBuf> = Vec::new();

    if let Some(ch) = &cade_home {
        let global_skills = ch.join("skills");
        if global_skills.exists() {
            watch_dirs.push(global_skills);
        }
    }

    let project_skills = cwd.join(".cade/skills");
    if project_skills.exists() {
        watch_dirs.push(project_skills.clone());
    }

    if watch_dirs.is_empty() {
        // No dirs to watch yet — still return the receiver; caller can start
        // without a watcher. The REPL will never receive on this channel.
        return rx;
    }

    std::thread::spawn(move || {
        // notify 6.x: create watcher with a sync std::sync::mpsc channel internally,
        // then forward to tokio channel via try_send.
        let (sync_tx, sync_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

        let mut watcher = match RecommendedWatcher::new(sync_tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("skill watcher: failed to create watcher: {e}");
                return;
            }
        };

        for dir in &watch_dirs {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                tracing::warn!("skill watcher: cannot watch {}: {e}", dir.display());
            } else {
                tracing::info!("skill watcher: watching {}", dir.display());
            }
        }

        // Forward relevant events to the tokio channel
        for res in sync_rx {
            match res {
                Ok(event) => {
                    let relevant = matches!(
                        event.kind,
                        EventKind::Create(CreateKind::File)
                            | EventKind::Create(CreateKind::Any)
                            | EventKind::Modify(ModifyKind::Data(_))
                            | EventKind::Modify(ModifyKind::Any)
                            | EventKind::Remove(RemoveKind::File)
                            | EventKind::Remove(RemoveKind::Any)
                    );
                    // Only care about SKILL.MD files
                    let is_skill_file = event.paths.iter().any(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.to_uppercase() == "SKILL.MD")
                            .unwrap_or(false)
                    });
                    if relevant && is_skill_file {
                        // Non-blocking send — drop if receiver is behind
                        let _ = tx.try_send(());
                    }
                }
                Err(e) => tracing::warn!("skill watcher error: {e}"),
            }
        }
    });

    rx
}

/// Convert a GitHub tree URL to a raw SKILL.MD URL.
/// https://github.com/USER/REPO/tree/BRANCH/path → https://raw.../path/SKILL.MD
pub fn github_url_to_raw_skill(url: &str) -> Option<String> {
    // Match: https://github.com/<user>/<repo>/tree/<branch>/<path...>
    let stripped = url
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/");
    let parts: Vec<&str> = stripped.splitn(5, '/').collect();
    if parts.len() >= 5 && parts[2] == "tree" {
        let user = parts[0];
        let repo = parts[1];
        let branch = parts[3];
        let path = parts[4];
        Some(format!(
            "https://raw.githubusercontent.com/{user}/{repo}/{branch}/{path}/SKILL.MD"
        ))
    } else if parts.len() >= 5 && parts[2] == "blob" {
        // Direct file URL — return as-is converted to raw
        let branch = parts[3];
        let path = parts[4..].join("/");
        Some(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            parts[0], parts[1], branch, path
        ))
    } else {
        None
    }
}

// endregion: --- Tests

/// Resolve a bare GitHub repo URL or `owner/repo` shorthand to a raw SKILL.md URL
/// when a specific `skill_name` is provided.
///
/// Supports:
/// - `https://github.com/owner/repo`  (bare repo URL)
/// - `https://github.com/owner/repo/` (with trailing slash)
/// - `owner/repo`                     (GitHub shorthand)
///
/// Combined with `skill_name`, resolves to:
/// `https://raw.githubusercontent.com/{owner}/{repo}/main/skills/{skill_name}/SKILL.md`
///
/// Returns `None` if:
/// - The URL is not a GitHub repo or shorthand
/// - No `skill_name` is provided (we can't guess which skill)
/// - The `skill_name` contains path traversal or invalid characters
pub fn resolve_github_repo_skill_url(source: &str, skill_name: Option<&str>) -> Option<String> {
    let skill = skill_name?;

    // Validate skill name: no path traversal, no slashes, alphanumeric + hyphens only
    if skill.is_empty()
        || skill.contains('/')
        || skill.contains('\\')
        || skill.contains("..")
        || !skill
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }

    // Try to extract owner/repo from the source
    let (owner, repo) = if source.contains("github.com") {
        // Full URL: https://github.com/owner/repo[/]
        let stripped = source
            .trim_start_matches("https://github.com/")
            .trim_start_matches("http://github.com/")
            .trim_end_matches('/');

        // Must be exactly owner/repo (no /tree/, /blob/, or deeper paths)
        let parts: Vec<&str> = stripped.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return None;
        }
        (parts[0], parts[1])
    } else if !source.contains("://") && !source.contains(' ') {
        // GitHub shorthand: owner/repo
        let parts: Vec<&str> = source.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return None;
        }
        (parts[0], parts[1])
    } else {
        return None;
    };

    Some(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/main/skills/{skill}/SKILL.md"
    ))
}

/// Write edited skill fields back to the SKILL.MD file on disk.
/// fields: [name, description, category, tags_csv, triggers_csv, body]
pub fn write_skill_to_disk(skill: &Skill, fields: &[String]) -> std::io::Result<()> {
    let name = &fields[0];
    let desc = &fields[1];
    let cat = &fields[2];
    let tags_str = &fields[3];
    let trig_str = &fields[4];
    let body = &fields[5];

    let fmt_list = |s: &str| -> String {
        let items: Vec<String> = s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if items.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                items
                    .iter()
                    .map(|t| format!("\"{}\"", t))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };

    let tags_yaml = fmt_list(tags_str);
    let trigs_yaml = fmt_list(trig_str);

    let content = format!(
        "---\nname: {name}\ndescription: {desc}\ncategory: {cat}\ntags: {tags_yaml}\ntriggers: {trigs_yaml}\n---\n\n{body}"
    );
    std::fs::write(&skill.path, content)
}

/// Download and install a skill from a URL into `target_dir/<skill-name>/SKILL.MD`.
/// Returns the installed skill on success.
///
/// Supports:
/// - Direct SKILL.MD URLs
/// - GitHub tree/blob URLs (resolved via `github_url_to_raw_skill`)
/// - Bare GitHub repo URLs or `owner/repo` shorthand with `skill_name`
///   (resolved via `resolve_github_repo_skill_url`)
///
/// Requires the `http` feature (enabled by default).
#[cfg(feature = "http")]
pub async fn install_skill_from_url(
    url: &str,
    target_dir: &Path,
    skill_name: Option<&str>,
) -> Result<Skill> {
    // 1. Try bare repo URL + skill_name first
    let resolved_from_repo = resolve_github_repo_skill_url(url, skill_name);

    // 2. Then try tree/blob URL conversion
    let raw_url = if let Some(resolved) = resolved_from_repo {
        resolved
    } else if url.contains("github.com") && (url.contains("/tree/") || url.contains("/blob/")) {
        github_url_to_raw_skill(url)
            .ok_or_else(|| Error::custom(format!("Cannot parse GitHub URL: {url}")))?
    } else if url.contains("github.com") && skill_name.is_none() {
        // Bare GitHub URL without --skill: we can't guess which skill
        return Err(Error::custom(format!(
            "Bare GitHub repo URL requires a 'skill' parameter to select which skill to install. \
             Example: install_skill(url=\"{url}\", skill=\"my-skill-name\")"
        )));
    } else {
        url.to_string()
    };

    // Derive skill ID from URL path
    let skill_id = raw_url
        .trim_end_matches("/SKILL.MD")
        .trim_end_matches("/SKILL.md")
        .rsplit('/')
        .next()
        .unwrap_or("downloaded-skill")
        .to_lowercase()
        .replace(' ', "-");

    // SEC-B4: Validate derived skill ID to prevent path traversal
    if !skill_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(Error::custom(format!(
            "Invalid skill ID derived from URL: {skill_id}"
        )));
    }

    let skill_dir = target_dir.join(&skill_id);
    let skill_file = skill_dir.join("SKILL.MD");

    if skill_file.exists() {
        return Err(Error::custom(format!(
            "Skill '{}' already installed at {}",
            skill_id,
            skill_file.display()
        )));
    }

    // Fetch content
    let client = reqwest::Client::new();
    let response = client
        .get(&raw_url)
        .header("User-Agent", "CADE-agent")
        .send()
        .await?
        .error_for_status()?;

    // Check content-type — reject obvious non-markdown responses
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if ct.contains("text/html") || ct.contains("application/json") {
        return Err(Error::custom(format!(
            "URL returned {ct} instead of skill markdown content. \
             The URL should serve raw SKILL.MD content (text/plain or text/markdown)."
        )));
    }

    let content = response.text().await?;

    // Validate content looks like a SKILL.MD file (frontmatter or markdown)
    let trimmed_content = content.trim();
    if trimmed_content.starts_with("<!DOCTYPE")
        || trimmed_content.starts_with("<html")
        || trimmed_content.starts_with("{\"")
    {
        return Err(Error::custom(
            "Response body looks like HTML or JSON, not a valid SKILL.MD file. \
             The URL should serve raw markdown content."
                .to_string(),
        ));
    }

    // Write
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(&skill_file, &content)?;

    // Parse and return
    let scope = SkillScope::Project; // installed to project scope by default
    parse_skill(&skill_id, &content, scope, skill_file)
}

// region:    --- Tests

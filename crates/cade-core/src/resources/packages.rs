/// Package manifest and install source types.
///
/// A CADE package bundles skills, prompt templates, themes, and subagent
/// definitions.  Packages are installed from npm, git, or local paths.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// region:    --- Types

/// The source location of a package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PackageSource {
    /// npm package spec, e.g. `"@foo/cade-tools"` or `"@foo/cade-tools@1.0.0"`.
    Npm { spec: String },
    /// Git repository URL with optional ref.
    Git { url: String, #[serde(skip_serializing_if = "Option::is_none")] rev: Option<String> },
    /// Absolute path on the local filesystem.
    Local { path: PathBuf },
}

impl PackageSource {
    /// Human-readable display string, suitable for CLI output.
    pub fn display_string(&self) -> String {
        match self {
            Self::Npm { spec } => format!("npm:{spec}"),
            Self::Git { url, rev: None } => format!("git:{url}"),
            Self::Git { url, rev: Some(r) } => format!("git:{url}@{r}"),
            Self::Local { path } => path.display().to_string(),
        }
    }

    /// Parse a source string from the CLI (`npm:...`, `git:...`, or path).
    pub fn parse(s: &str) -> crate::Result<Self> {
        if let Some(spec) = s.strip_prefix("npm:") {
            return Ok(Self::Npm { spec: spec.to_string() });
        }
        if let Some(rest) = s.strip_prefix("git:") {
            let (url, rev) = split_rev(rest);
            return Ok(Self::Git { url: url.to_string(), rev });
        }
        if s.starts_with("https://") || s.starts_with("http://") || s.starts_with("ssh://") {
            let (url, rev) = split_rev(s);
            return Ok(Self::Git { url: url.to_string(), rev });
        }
        // Local path
        let path = PathBuf::from(s);
        Ok(Self::Local { path })
    }

    /// Derive a stable on-disk directory name for this source.
    pub fn dir_name(&self) -> String {
        match self {
            Self::Npm { spec } => {
                spec.replace(['@', '/', ':'], "_")
            }
            Self::Git { url, rev } => {
                let base = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_start_matches("ssh://")
                    .trim_start_matches("git://")
                    .replace(['/', ':'], "_");
                match rev {
                    Some(r) => format!("{base}_{r}"),
                    None => base,
                }
            }
            Self::Local { path } => {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("local")
                    .to_string()
            }
        }
    }
}

/// Scope of a package installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageScope {
    /// Installed to `~/.cade/packages/` and available to all projects.
    Global,
    /// Installed to `.cade/packages/` and available only in this project.
    Project,
}

/// Parsed package manifest (`package.json` or `cade-package.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    // Resource directories/files to load from this package
    #[serde(default)]
    pub skills: Vec<PathBuf>,
    #[serde(default)]
    pub prompts: Vec<PathBuf>,
    #[serde(default)]
    pub themes: Vec<PathBuf>,
    #[serde(default)]
    pub subagents: Vec<PathBuf>,
    /// JSON schema tool definition files.
    #[serde(default)]
    pub tools: Vec<PathBuf>,
}

// endregion: --- Types

// region:    --- Discovery

/// Derive the installation root for a package given its scope and agent dir.
pub fn package_root(source: &PackageSource, scope: PackageScope, agent_dir: &Path) -> PathBuf {
    let base = match scope {
        PackageScope::Global  => agent_dir.join("packages"),
        PackageScope::Project => {
            // Caller should set cwd-relative .cade/packages; we don't have cwd here.
            // Return agent_dir-relative as fallback.
            agent_dir.join("packages")
        }
    };
    base.join(source.dir_name())
}

/// Load a package manifest from a directory.
///
/// Looks for (in order): `cade-package.json`, `package.json`.
/// If neither is found, auto-discovers resources from conventional directories.
pub fn load_manifest(root: &Path) -> crate::Result<PackageManifest> {
    let candidates = ["cade-package.json", "package.json"];
    for name in &candidates {
        let path = root.join(name);
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| crate::Error::custom(format!("read {}: {e}", path.display())))?;
            let manifest: PackageManifest = serde_json::from_str(&content)
                .map_err(|e| crate::Error::custom(format!("parse {}: {e}", path.display())))?;
            return Ok(manifest);
        }
    }
    // Auto-discover
    Ok(auto_discover_manifest(root))
}

/// Auto-discover package resources from conventional directories when no
/// manifest file is present.
fn auto_discover_manifest(root: &Path) -> PackageManifest {
    let mut m = PackageManifest::default();
    m.name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    for (dir, field) in &[
        ("skills",   "skills"),
        ("prompts",  "prompts"),
        ("themes",   "themes"),
        ("subagents","subagents"),
        ("tools",    "tools"),
    ] {
        let p = root.join(dir);
        if p.exists() {
            let paths = vec![p];
            match *field {
                "skills"    => m.skills    = paths,
                "prompts"   => m.prompts   = paths,
                "themes"    => m.themes    = paths,
                "subagents" => m.subagents = paths,
                "tools"     => m.tools     = paths,
                _ => {}
            }
        }
    }
    m
}

// endregion: --- Discovery

// region:    --- Support

fn split_rev(s: &str) -> (&str, Option<String>) {
    // Look for `@` after the protocol part to find rev
    // e.g. "github.com/user/repo@v1" or "https://github.com/user/repo@v1"
    if let Some(at) = s.rfind('@') {
        // Don't mistake the `@` in `git@github.com` for a rev separator
        let before = &s[..at];
        // If the @ is in a host position (git@...) skip it
        if !before.contains('/') {
            return (s, None);
        }
        return (&s[..at], Some(s[at + 1..].to_string()));
    }
    (s, None)
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_source_parse_npm() {
        // -- Exec
        let src = PackageSource::parse("npm:@foo/bar@1.0.0").unwrap();

        // -- Check
        assert_eq!(src, PackageSource::Npm { spec: "@foo/bar@1.0.0".to_string() });
    }

    #[test]
    fn test_package_source_parse_git() {
        // -- Exec
        let src = PackageSource::parse("git:github.com/user/repo@v1").unwrap();

        // -- Check
        assert_eq!(src, PackageSource::Git {
            url: "github.com/user/repo".to_string(),
            rev: Some("v1".to_string()),
        });
    }

    #[test]
    fn test_package_source_parse_local() {
        // -- Exec
        let src = PackageSource::parse("/home/user/my-package").unwrap();

        // -- Check
        assert_eq!(src, PackageSource::Local { path: PathBuf::from("/home/user/my-package") });
    }

    #[test]
    fn test_package_source_display() {
        // -- Setup & Fixtures
        let src = PackageSource::Npm { spec: "@foo/bar".to_string() };

        // -- Check
        assert_eq!(src.display_string(), "npm:@foo/bar");
    }

    #[test]
    fn test_package_source_dir_name_npm() {
        // -- Setup & Fixtures
        let src = PackageSource::Npm { spec: "@foo/cade-tools".to_string() };

        // -- Check
        let name = src.dir_name();
        assert!(!name.contains('@'));
        assert!(!name.contains('/'));
    }

    #[test]
    fn test_load_manifest_auto_discover() {
        // -- Setup & Fixtures
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();

        // -- Exec
        let manifest = load_manifest(dir.path()).unwrap();

        // -- Check
        assert!(!manifest.skills.is_empty());
    }
}

// endregion: --- Tests

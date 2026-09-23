use crate::manifest::PluginManifest;
use crate::marketplace::compute_sha256;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginValidationReport {
    pub is_valid: bool,
    pub name: String,
    pub version: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedPlugin {
    pub archive_path: PathBuf,
    pub sha256: String,
    pub file_size_bytes: u64,
}

/// Scaffolds a compliant CADE plugin directory structure.
pub fn init_plugin(dir: &Path, name: &str, use_toml: bool) -> crate::Result<PathBuf> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        return Err(crate::Error::custom("Plugin name cannot be empty"));
    }

    let plugin_dir = dir.join(clean_name);
    if plugin_dir.exists() {
        return Err(crate::Error::custom(format!(
            "Directory already exists at {}",
            plugin_dir.display()
        )));
    }

    std::fs::create_dir_all(&plugin_dir)?;
    std::fs::create_dir_all(plugin_dir.join("skills").join("hello"))?;

    // Manifest
    if use_toml {
        let toml_content = format!(
            r#"# CADE Plugin Manifest
name = "{clean_name}"
version = "0.1.0"
description = "A sample CADE plugin"
author = "Your Name <you@example.com>"

skills = [
    "skills/hello/SKILL.md"
]
"#
        );
        std::fs::write(plugin_dir.join("cade-plugin.toml"), toml_content)?;
    } else {
        let json_content = serde_json::json!({
            "$schema": "https://cade.dev/schemas/cade-plugin.v1.json",
            "name": clean_name,
            "version": "0.1.0",
            "description": "A sample CADE plugin",
            "author": "Your Name <you@example.com>",
            "skills": [
                "skills/hello/SKILL.md"
            ]
        });
        std::fs::write(
            plugin_dir.join("cade-plugin.json"),
            serde_json::to_string_pretty(&json_content)?,
        )?;
    }

    // Sample skill
    let skill_md = format!(
        r#"---
name: hello
description: A sample skill provided by the {clean_name} plugin.
---

# Hello Skill

When the user asks for a greeting, respond with a welcoming greeting.
"#
    );
    std::fs::write(
        plugin_dir.join("skills").join("hello").join("SKILL.md"),
        skill_md,
    )?;

    // README
    let readme = format!(
        r#"# {clean_name}

A CADE plugin.

## Installation
Run:
```bash
cade plugin install .
```
"#
    );
    std::fs::write(plugin_dir.join("README.md"), readme)?;

    Ok(plugin_dir)
}

/// Validates a plugin directory, ensuring manifest existence, valid metadata, and existing referenced paths.
pub fn validate_plugin(root: &Path) -> crate::Result<PluginValidationReport> {
    if !root.is_dir() {
        return Err(crate::Error::custom(format!(
            "{} is not a directory",
            root.display()
        )));
    }

    let manifest = match PluginManifest::load(root) {
        Ok(m) => m,
        Err(e) => {
            return Ok(PluginValidationReport {
                is_valid: false,
                name: "unknown".into(),
                version: "unknown".into(),
                issues: vec![format!("Failed to load plugin manifest: {e}")],
            });
        }
    };

    let mut issues = Vec::new();

    if manifest.name.trim().is_empty() || manifest.name == "unknown" {
        issues.push("Plugin manifest must declare a non-empty 'name'".into());
    }

    let version_str = manifest
        .version
        .clone()
        .unwrap_or_else(|| "0.1.0".to_string());

    if version_str.trim().is_empty() {
        issues.push("Plugin manifest must declare a valid 'version'".into());
    }

    // Check declared skills
    for skill_path in &manifest.skills {
        let full = root.join(skill_path);
        if !full.exists() {
            issues.push(format!(
                "Declared skill path does not exist: {}",
                skill_path.display()
            ));
        }
    }

    // Check declared subagents
    for subagent_path in &manifest.subagents {
        let full = root.join(subagent_path);
        if !full.exists() {
            issues.push(format!(
                "Declared subagent path does not exist: {}",
                subagent_path.display()
            ));
        }
    }

    // Check declared themes
    for theme_path in &manifest.themes {
        let full = root.join(theme_path);
        if !full.exists() {
            issues.push(format!(
                "Declared theme path does not exist: {}",
                theme_path.display()
            ));
        }
    }

    let is_valid = issues.is_empty();

    Ok(PluginValidationReport {
        is_valid,
        name: manifest.name,
        version: version_str,
        issues,
    })
}

/// Packs a validated plugin into a .tar.gz archive and computes its SHA-256 hash.
pub fn pack_plugin(root: &Path, output_path: Option<&Path>) -> crate::Result<PackedPlugin> {
    let report = validate_plugin(root)?;
    if !report.is_valid {
        return Err(crate::Error::custom(format!(
            "Plugin validation failed: {}",
            report.issues.join("; ")
        )));
    }

    let archive_name = format!("{}-{}.tar.gz", report.name, report.version);
    let target_file = match output_path {
        Some(p) => {
            if p.is_dir() {
                p.join(archive_name)
            } else {
                p.to_path_buf()
            }
        }
        None => root.join(&archive_name),
    };

    if let Some(parent) = target_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs::File;
    use tar::Builder;

    let file = File::create(&target_file)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(enc);

    // Recursively append root files, excluding .git, target, and the output archive itself
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|e| crate::Error::custom(e.to_string()))?;
        let path = entry.path();

        // Skip target file if inside root
        if path == target_file {
            continue;
        }

        let rel = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = rel.to_string_lossy();
        if rel_str.starts_with(".git")
            || rel_str.starts_with("target")
            || rel_str.ends_with(".tar.gz")
        {
            continue;
        }

        if path.is_dir() {
            builder.append_dir(rel, path)?;
        } else if path.is_file() {
            let mut f = File::open(path)?;
            builder.append_file(rel, &mut f)?;
        }
    }

    let enc = builder.into_inner()?;
    enc.finish()?;

    // Read archive bytes to compute SHA-256 and size
    let bytes = std::fs::read(&target_file)?;
    let sha256 = compute_sha256(&bytes);
    let file_size_bytes = bytes.len() as u64;

    Ok(PackedPlugin {
        archive_path: target_file,
        sha256,
        file_size_bytes,
    })
}

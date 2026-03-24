/// Package management commands: install, remove, list, update.
use std::path::Path;

use cade_core::resources::packages::{PackageScope, PackageSource, load_manifest};
use cade_core::settings::SettingsManager;

use crate::Result;

// region:    --- Commands

/// `cade package install <source> [-l]`
pub async fn cmd_install(
    source_str: &str,
    project_local: bool,
    settings: &mut SettingsManager,
    cwd: &Path,
    agent_dir: &Path,
) -> Result<()> {
    let source = PackageSource::parse(source_str)
        .map_err(|e| crate::Error::custom(format!("invalid package source '{source_str}': {e}")))?;

    let scope = if project_local { PackageScope::Project } else { PackageScope::Global };

    let install_base = match scope {
        PackageScope::Global  => agent_dir.join("packages"),
        PackageScope::Project => cwd.join(".cade").join("packages"),
    };

    let dest = install_base.join(source.dir_name());

    if dest.exists() {
        println!("Package '{}' is already installed at {}", source.display_string(), dest.display());
        return Ok(());
    }

    println!("Installing {}…", source.display_string());
    install_package(&source, &dest).await?;

    // Load and display the manifest
    match load_manifest(&dest) {
        Ok(m) => {
            let name = if m.name.is_empty() { source.display_string() } else { m.name.clone() };
            println!("✓ Installed package '{name}'");
            if !m.skills.is_empty()    { println!("  {} skill(s)",    m.skills.len()); }
            if !m.prompts.is_empty()   { println!("  {} prompt(s)",   m.prompts.len()); }
            if !m.themes.is_empty()    { println!("  {} theme(s)",    m.themes.len()); }
            if !m.subagents.is_empty() { println!("  {} subagent(s)", m.subagents.len()); }
        }
        Err(e) => tracing::warn!("Could not read package manifest: {e}"),
    }

    // Persist to settings
    let sources = match scope {
        PackageScope::Global => &mut settings.global_settings_mut().packages,
        PackageScope::Project => {
            // Project settings don't have packages yet — log only
            println!("Note: project-local package tracking not yet persisted.");
            return Ok(());
        }
    };
    if !sources.iter().any(|s| s == &source) {
        sources.push(source);
        settings.save_global()
            .map_err(|e| crate::Error::custom(format!("save settings: {e}")))?;
    }

    Ok(())
}

/// `cade package remove <source>`
pub fn cmd_remove(source_str: &str, agent_dir: &Path) -> Result<()> {
    let source = PackageSource::parse(source_str)
        .map_err(|e| crate::Error::custom(format!("invalid package source: {e}")))?;
    let dest = agent_dir.join("packages").join(source.dir_name());
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| crate::Error::custom(format!("remove {}: {e}", dest.display())))?;
        println!("✓ Removed '{}'", source.display_string());
    } else {
        println!("Package '{}' not found at {}", source.display_string(), dest.display());
    }
    Ok(())
}

/// `cade package list`
pub fn cmd_list(agent_dir: &Path) -> Result<()> {
    let packages_dir = agent_dir.join("packages");
    if !packages_dir.exists() {
        println!("No packages installed.");
        return Ok(());
    }
    let entries: Vec<_> = std::fs::read_dir(&packages_dir)
        .map_err(|e| crate::Error::custom(format!("read packages dir: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    println!("Installed packages:");
    for entry in entries {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match load_manifest(&path) {
            Ok(m) if !m.name.is_empty() => println!("  {} ({})", m.name, name),
            _ => println!("  {}", name),
        }
    }
    Ok(())
}

/// `cade package update`
pub async fn cmd_update(agent_dir: &Path) -> Result<()> {
    let packages_dir = agent_dir.join("packages");
    if !packages_dir.exists() {
        println!("No packages to update.");
        return Ok(());
    }
    println!("Checking for updates…");
    // For now, just print what's installed. Full update logic (git pull, npm update)
    // can be added when package provenance tracking is implemented.
    cmd_list(agent_dir)?;
    println!("(Full update support coming soon.)");
    Ok(())
}

// endregion: --- Commands

// region:    --- Install helpers

async fn install_package(source: &PackageSource, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::Error::custom(format!("create packages dir: {e}")))?;
    }

    match source {
        PackageSource::Local { path } => {
            if path.is_dir() {
                // Copy the directory
                copy_dir_all(path, dest)?;
            } else if path.is_file() {
                // Single file — treat as a plugin manifest or skill
                std::fs::create_dir_all(dest)
                    .map_err(|e| crate::Error::custom(format!("mkdir: {e}")))?;
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("package.json");
                std::fs::copy(path, dest.join(filename))
                    .map_err(|e| crate::Error::custom(format!("copy: {e}")))?;
            } else {
                return Err(crate::Error::custom(format!("path does not exist: {}", path.display())));
            }
        }
        PackageSource::Git { url, rev } => {
            install_git(url, rev.as_deref(), dest).await?;
        }
        PackageSource::Npm { spec } => {
            install_npm(spec, dest).await?;
        }
    }
    Ok(())
}

async fn install_git(url: &str, rev: Option<&str>, dest: &Path) -> Result<()> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone").arg("--depth=1");
    if let Some(r) = rev {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(url).arg(dest);
    let out = cmd.output().await
        .map_err(|e| crate::Error::custom(format!("git clone: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(crate::Error::custom(format!("git clone failed: {stderr}")));
    }
    // Run npm install if package.json present
    if dest.join("package.json").exists() {
        let _ = tokio::process::Command::new("npm")
            .arg("install")
            .current_dir(dest)
            .output()
            .await;
    }
    Ok(())
}

async fn install_npm(spec: &str, dest: &Path) -> Result<()> {
    // Temporary dir approach: npm pack + extract
    let tmp = tempfile::tempdir()
        .map_err(|e| crate::Error::custom(format!("tempdir: {e}")))?;
    let out = tokio::process::Command::new("npm")
        .arg("install")
        .arg("--prefix")
        .arg(tmp.path())
        .arg(spec)
        .output()
        .await
        .map_err(|e| crate::Error::custom(format!("npm install: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(crate::Error::custom(format!("npm install failed: {stderr}")));
    }
    // Find the installed package under node_modules
    let node_modules = tmp.path().join("node_modules");
    // Package name without version
    let pkg_name = spec.split('@').next().unwrap_or(spec);
    let src = node_modules.join(pkg_name);
    if src.exists() {
        copy_dir_all(&src, dest)?;
    } else {
        // Try to find it
        if let Ok(entries) = std::fs::read_dir(&node_modules) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    copy_dir_all(&entry.path(), dest)?;
                    break;
                }
            }
        }
    }
    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .map_err(|e| crate::Error::custom(format!("mkdir {}: {e}", dst.display())))?;
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let dest_path = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path).ok();
        } else if entry.file_type().is_file() {
            if let Some(p) = dest_path.parent() {
                std::fs::create_dir_all(p).ok();
            }
            std::fs::copy(entry.path(), &dest_path)
                .map_err(|e| crate::Error::custom(format!("copy {}: {e}", entry.path().display())))?;
        }
    }
    Ok(())
}

// endregion: --- Install helpers

use serde::{Deserialize, Serialize};

/// The root index.json format hosted by the central Plugin Registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub version: String,
    pub plugins: Vec<RegistryPluginInfo>,
}

/// Metadata for a single plugin in the registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPluginInfo {
    pub id: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// URL to a compressed archive (.tar.gz) or git repo containing the plugin
    pub url: String,
    /// Optional expected SHA-256 checksum of the archive for integrity verification
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Compute the SHA-256 hex digest of a byte slice.
pub fn compute_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fetches and installs a plugin from a .tar.gz URL without checksum verification.
pub async fn install_plugin(
    url: &str,
    plugin_id: &str,
    target_dir: &std::path::Path,
) -> crate::Result<crate::manifest::PluginManifest> {
    install_plugin_with_checksum(url, plugin_id, target_dir, None).await
}

/// Fetches and installs a plugin from a .tar.gz URL, validating its SHA-256 checksum before extracting.
pub async fn install_plugin_with_checksum(
    url: &str,
    plugin_id: &str,
    target_dir: &std::path::Path,
    expected_sha256: Option<&str>,
) -> crate::Result<crate::manifest::PluginManifest> {
    validate_plugin_id(plugin_id)?;
    // 1. Download tarball
    let resp = reqwest::get(url)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| crate::Error::custom(e.to_string()))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| crate::Error::custom(e.to_string()))?;

    // 1b. Verify SHA-256 checksum if provided
    if let Some(expected) = expected_sha256 {
        let actual = compute_sha256(&bytes);
        let expected_clean = expected.trim().to_lowercase();
        if actual != expected_clean {
            return Err(crate::Error::IntegrityError {
                expected: expected_clean,
                actual,
            });
        }
    }

    activate_archive(&bytes, plugin_id, target_dir)
}

/// Stable IDs are single directory names, shared by installation and removal.
pub fn validate_plugin_id(plugin_id: &str) -> crate::Result<()> {
    if plugin_id.is_empty()
        || !plugin_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(crate::Error::custom(
            "Invalid plugin ID: use letters, numbers, hyphens and underscores",
        ));
    }
    Ok(())
}

fn activate_archive(
    bytes: &[u8],
    plugin_id: &str,
    target_dir: &std::path::Path,
) -> crate::Result<crate::manifest::PluginManifest> {
    validate_plugin_id(plugin_id)?;
    std::fs::create_dir_all(target_dir)?;
    let staging = target_dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(staging.clone());

    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let tar = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(tar);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(crate::Error::custom(
                "Plugin archives may contain only regular files and directories",
            ));
        }
        if !entry.unpack_in(&staging)? {
            return Err(crate::Error::custom("Plugin archive path escapes package"));
        }
    }

    // 2b. Directory Promotion (Option B): promote single nested subdirectories to root
    let paths = std::fs::read_dir(&staging)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let package = if paths.len() == 1 && paths[0].is_dir() {
        &paths[0]
    } else {
        &staging
    };
    if !["cade-plugin.toml", "cade-plugin.json", "package.json"]
        .iter()
        .any(|name| package.join(name).is_file())
    {
        return Err(crate::Error::custom("Plugin archive requires a manifest"));
    }
    let report = crate::dev::validate_plugin(package)?;
    if !report.is_valid {
        return Err(crate::Error::custom(format!(
            "Plugin validation failed: {}",
            report.issues.join("; ")
        )));
    }
    let manifest = crate::manifest::PluginManifest::load(package)?;
    let plugin_dir = target_dir.join(plugin_id);
    let backup = target_dir.join(format!(".backup-{}", uuid::Uuid::new_v4()));
    let replacing = plugin_dir.exists();
    if replacing {
        std::fs::rename(&plugin_dir, &backup)?;
    }
    if let Err(error) = std::fs::rename(package, &plugin_dir) {
        if replacing {
            std::fs::rename(&backup, &plugin_dir)?;
        }
        return Err(error.into());
    }
    if replacing {
        // Activation has committed; a cleanup error must not report a failed install.
        if let Err(error) = std::fs::remove_dir_all(&backup) {
            tracing::warn!(%error, "failed to remove replaced plugin backup");
        }
    }

    // 3. Load manifest
    Ok(manifest)
}

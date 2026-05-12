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
}

/// Fetches and installs a plugin from a .tar.gz URL.
pub async fn install_plugin(
    url: &str,
    plugin_id: &str,
    target_dir: &std::path::Path,
) -> crate::Result<crate::manifest::PluginManifest> {
    // 1. Download tarball
    let resp = reqwest::get(url).await.map_err(|e| crate::Error::custom(e.to_string()))?;
    let bytes = resp.bytes().await.map_err(|e| crate::Error::custom(e.to_string()))?;

    // 2. Unpack tarball to target_dir/plugin_id
    let plugin_dir = target_dir.join(plugin_id.replace('/', "_"));
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir).ok();
    }
    std::fs::create_dir_all(&plugin_dir).map_err(|e| crate::Error::custom(e.to_string()))?;

    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let tar = GzDecoder::new(Cursor::new(bytes.as_ref()));
    let mut archive = Archive::new(tar);
    archive.unpack(&plugin_dir).map_err(|e| crate::Error::custom(format!("Unpack failed: {e}")))?;

    // 3. Load manifest
    crate::manifest::PluginManifest::load(&plugin_dir)
}

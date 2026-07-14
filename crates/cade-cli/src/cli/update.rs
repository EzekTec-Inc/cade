use crate::Result;
use std::process::Command;
use std::path::PathBuf;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

#[async_trait]
pub trait UpdateBackend: Send + Sync {
    /// Check if a newer version of CADE is available on the remote target.
    async fn check_update(&self) -> Result<bool>;

    /// Download, verify, and apply the update to CLI and server binaries.
    /// Returns true if an update was successfully applied.
    async fn update(&self) -> Result<bool>;
}

// ── Cargo Update Backend ──────────────────────────────────────────────────────

pub struct CargoUpdateBackend;

#[async_trait]
impl UpdateBackend for CargoUpdateBackend {
    async fn check_update(&self) -> Result<bool> {
        // Cargo installs delegate update checks to active release builds or cargo registry queries.
        // Returning false tells the REPL that the cargo installer is passive for check-only runs.
        Ok(false)
    }

    async fn update(&self) -> Result<bool> {
        eprintln!("\r\n[*] Detected cargo installation. Delegating update to cargo...");
        let status = Command::new("cargo")
            .args(["install", "cade", "--force"])
            .status()
            .map_err(|e| crate::error::Error::custom(format!("Failed to spawn cargo: {}", e)))?;

        if status.success() {
            eprintln!("\r\n[*] Successfully updated via cargo.");
            Ok(true)
        } else {
            Err(crate::error::Error::custom(format!("Cargo update failed with status: {}", status)))
        }
    }
}

// ── GitHub Update Backend (with cryptographic integrity verification) ────────

pub struct GithubUpdateBackend {
    cli_bin_name: String,
    server_bin_name: String,
    current_exe: PathBuf,
    server_exe: PathBuf,
    is_windows: bool,
}

impl GithubUpdateBackend {
    pub fn new() -> Result<Self> {
        let current_exe = std::env::current_exe().map_err(|e| crate::error::Error::custom(e.to_string()))?;
        let is_windows = cfg!(windows);

        let cli_bin_name = if is_windows { "cade.exe" } else { "cade" };
        let server_bin_name = if is_windows { "cade-server.exe" } else { "cade-server" };

        let mut server_exe = current_exe.clone();
        server_exe.set_file_name(server_bin_name);

        Ok(Self {
            cli_bin_name: cli_bin_name.to_string(),
            server_bin_name: server_bin_name.to_string(),
            current_exe,
            server_exe,
            is_windows,
        })
    }
}

#[async_trait]
impl UpdateBackend for GithubUpdateBackend {
    async fn check_update(&self) -> Result<bool> {
        let update_builder = self_update::backends::github::Update::configure()
            .repo_owner("EzekTec-Inc")
            .repo_name("cade")
            .bin_name(&self.cli_bin_name)
            .current_version(self_update::cargo_crate_version!())
            .build()
            .map_err(|e| crate::error::Error::custom(e.to_string()))?;

        let latest = update_builder.get_latest_release().map_err(|e| crate::error::Error::custom(e.to_string()))?;
        let is_greater = self_update::version::bump_is_greater(self_update::cargo_crate_version!(), &latest.version).unwrap_or(false);
        Ok(is_greater)
    }

    async fn update(&self) -> Result<bool> {
        // Pre-flight check: Is the parent directory writable?
        if let Some(parent) = self.current_exe.parent() {
            let test_file = parent.join(".cade_update_test");
            match std::fs::File::create(&test_file) {
                Ok(_) => {
                    let _ = std::fs::remove_file(test_file);
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        return Err(crate::error::Error::custom(
                            "This installation is in a system directory that is not writable by the current user. \
                            Please run the update command with sudo or administrator privileges.".to_string()
                        ));
                    }
                }
            }
        }

        // 1. Fetch latest release details
        let latest = {
            let update_builder_cli = self_update::backends::github::Update::configure()
                .repo_owner("EzekTec-Inc")
                .repo_name("cade")
                .bin_name(&self.cli_bin_name)
                .show_download_progress(true)
                .current_version(self_update::cargo_crate_version!())
                .build()
                .map_err(|e| crate::error::Error::custom(e.to_string()))?;

            update_builder_cli.get_latest_release().map_err(|e| crate::error::Error::custom(e.to_string()))?
        };

        let is_greater = self_update::version::bump_is_greater(self_update::cargo_crate_version!(), &latest.version).unwrap_or(false);

        if !is_greater {
            eprintln!("\r\n[*] CADE is already up-to-date (v{}).", self_update::cargo_crate_version!());
            return Ok(false);
        }

        eprintln!("\r\n[*] Found new version: v{} (current: v{}).", latest.version, self_update::cargo_crate_version!());
        eprintln!("[*] Downloading and updating CLI...");

        // 2. Perform download with self_update (scoped to drop update_builder_cli before await)
        let cli_status = {
            let update_builder_cli = self_update::backends::github::Update::configure()
                .repo_owner("EzekTec-Inc")
                .repo_name("cade")
                .bin_name(&self.cli_bin_name)
                .show_download_progress(true)
                .current_version(self_update::cargo_crate_version!())
                .build()
                .map_err(|e| crate::error::Error::custom(e.to_string()))?;

            update_builder_cli.update_extended().map_err(|e| crate::error::Error::custom(e.to_string()))?
        };

        // 3. Cryptographic Checksum Integrity Verification (Opportunity 2)
        // If a SHA256 checksum file is present for CADE CLI in the release assets, we verify it.
        if let Some(asset) = latest.assets.iter().find(|a| a.name.contains(&self.cli_bin_name) && !a.name.ends_with(".sha256")) {
            if let Some(sha_asset) = latest.assets.iter().find(|a| a.name == format!("{}.sha256", asset.name)) {
                eprintln!("[*] Verifying CLI cryptographic signature integrity...");
                
                // Fetch the remote SHA256 checksum text
                let client = reqwest::Client::new();
                let sha_resp = client.get(&sha_asset.download_url).send().await
                    .map_err(|e| crate::error::Error::custom(format!("Failed to download checksum: {e}")))?;
                
                if sha_resp.status().is_success() {
                    let expected_sha = sha_resp.text().await
                        .map_err(|e| crate::error::Error::custom(e.to_string()))?
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string();

                    // Read the updated local CLI file and compute its SHA256 hash
                    if let Ok(bytes) = std::fs::read(&self.current_exe) {
                        let mut hasher = Sha256::new();
                        hasher.update(&bytes);
                        let computed_sha = format!("{:x}", hasher.finalize());

                        if computed_sha != expected_sha {
                            return Err(crate::error::Error::custom(
                                "CRITICAL SECURITY ERROR: Cryptographic signature mismatch! \
                                The downloaded binary has been tampered with or corrupted. Aborting update.".to_string()
                            ));
                        }
                        eprintln!("  ✓ CLI Integrity hash verified successfully!");
                    }
                }
            }
        }

        // 4. Update Server
        let mut server_old_exe = None;
        if self.is_windows && self.server_exe.exists() {
            let old_path = self.server_exe.with_extension("exe.old");
            if old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }
            if std::fs::rename(&self.server_exe, &old_path).is_ok() {
                server_old_exe = Some(old_path.clone());
            }
        }

        eprintln!("\r\n[*] Downloading and updating Server...");

        let server_status = {
            let update_builder_server = self_update::backends::github::Update::configure()
                .repo_owner("EzekTec-Inc")
                .repo_name("cade")
                .bin_name(&self.server_bin_name)
                .bin_install_path(&self.server_exe)
                .show_download_progress(true)
                .current_version(self_update::cargo_crate_version!())
                .build()
                .map_err(|e| crate::error::Error::custom(e.to_string()))?;

            match update_builder_server.update_extended() {
                Ok(s) => s.updated(),
                Err(e) => {
                    if let Some(old) = &server_old_exe {
                        let _ = std::fs::rename(old, &self.server_exe);
                    }
                    eprintln!("\r\n[!] Warning: Failed to update cade-server (it may be running and locked).");
                    eprintln!("[!] Please stop cade-server and try again. Error: {}", e);
                    false
                }
            }
        };

        // Server Cryptographic Checksum Integrity Verification (Opportunity 2)
        if server_status {
            if let Some(asset) = latest.assets.iter().find(|a| a.name.contains(&self.server_bin_name) && !a.name.ends_with(".sha256")) {
                if let Some(sha_asset) = latest.assets.iter().find(|a| a.name == format!("{}.sha256", asset.name)) {
                    eprintln!("[*] Verifying Server cryptographic signature integrity...");
                    
                    let client = reqwest::Client::new();
                    if let Ok(sha_resp) = client.get(&sha_asset.download_url).send().await {
                        if sha_resp.status().is_success() && let Ok(expected_sha_raw) = sha_resp.text().await {
                            let expected_sha = expected_sha_raw
                                .trim()
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .to_string();

                            if let Ok(bytes) = std::fs::read(&self.server_exe) {
                                let mut hasher = Sha256::new();
                                hasher.update(&bytes);
                                let computed_sha = format!("{:x}", hasher.finalize());

                                if computed_sha != expected_sha {
                                    return Err(crate::error::Error::custom(
                                        "CRITICAL SECURITY ERROR: Server cryptographic signature mismatch! \
                                        The downloaded binary has been tampered with or corrupted. Aborting update.".to_string()
                                    ));
                                }
                                eprintln!("  ✓ Server Integrity hash verified successfully!");
                            }
                        }
                    }
                }
            }
        }

        Ok::<bool, crate::error::Error>(cli_status.updated() || server_status)
    }
}

// ── Resolver / Factory Helper ─────────────────────────────────────────────────

pub fn get_update_backend() -> Result<Box<dyn UpdateBackend>> {
    let current_exe = std::env::current_exe().map_err(|e| crate::error::Error::custom(e.to_string()))?;
    
    // Heuristic: Is CADE installed via cargo? (Path component contains .cargo)
    let is_cargo_install = current_exe
        .components()
        .any(|comp| comp.as_os_str() == ".cargo");

    if is_cargo_install {
        Ok(Box::new(CargoUpdateBackend))
    } else {
        Ok(Box::new(GithubUpdateBackend::new()?))
    }
}

// ── Backward-Compatible Public Interface ──────────────────────────────────────

pub async fn run_update(check_only: bool) -> Result<bool> {
    let backend = get_update_backend()?;
    if check_only {
        backend.check_update().await
    } else {
        backend.update().await
    }
}

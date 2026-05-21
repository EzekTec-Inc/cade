use crate::Result;
use std::process::Command;

pub async fn run_update(check_only: bool) -> Result<bool> {
    let status = tokio::task::spawn_blocking(move || {
        let current_exe = std::env::current_exe().map_err(|e| crate::error::Error::custom(e.to_string()))?;
        let is_windows = cfg!(windows);

        let cli_bin_name = if is_windows { "cade.exe" } else { "cade" };
        let server_bin_name = if is_windows { "cade-server.exe" } else { "cade-server" };

        let mut server_exe = current_exe.clone();
        server_exe.set_file_name(server_bin_name);

        // Pre-flight check: Is the directory writable?
        if let Some(parent) = current_exe.parent() {
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

        let update_builder_cli = self_update::backends::github::Update::configure()
            .repo_owner("EzekTec-Inc")
            .repo_name("cade")
            .bin_name(cli_bin_name)
            .show_download_progress(true)
            .current_version(self_update::cargo_crate_version!())
            .build()
            .map_err(|e| crate::error::Error::custom(e.to_string()))?;

        if check_only {
            let latest = update_builder_cli.get_latest_release().map_err(|e| crate::error::Error::custom(e.to_string()))?;
            let is_greater = self_update::version::bump_is_greater(self_update::cargo_crate_version!(), &latest.version).unwrap_or(false);
            return Ok::<bool, crate::error::Error>(is_greater);
        }

        // Detection: Was CADE installed via Cargo?
        // A common heuristic: The executable path contains ".cargo/bin" or ".cargo\bin"
        let is_cargo_install = current_exe
            .components()
            .any(|comp| comp.as_os_str() == ".cargo");

        if is_cargo_install {
            eprintln!("\r\n[*] Detected cargo installation. Delegating update to cargo...");
            let status = Command::new("cargo")
                .args(["install", "cade", "--force"])
                .status()
                .map_err(|e| crate::error::Error::custom(format!("Failed to spawn cargo: {}", e)))?;

            if status.success() {
                eprintln!("\r\n[*] Successfully updated via cargo.");
                return Ok(true);
            } else {
                return Err(crate::error::Error::custom(format!("Cargo update failed with status: {}", status)));
            }
        }

        eprintln!("\r\n[*] Downloading and updating CLI...");
        let cli_status = update_builder_cli.update_extended().map_err(|e| crate::error::Error::custom(e.to_string()))?;

        let update_builder_server = self_update::backends::github::Update::configure()
            .repo_owner("EzekTec-Inc")
            .repo_name("cade")
            .bin_name(server_bin_name)
            .bin_install_path(&server_exe)
            .show_download_progress(true)
            .current_version(self_update::cargo_crate_version!())
            .build()
            .map_err(|e| crate::error::Error::custom(e.to_string()))?;

        eprintln!("\r\n[*] Downloading and updating Server...");

        // Windows Quarantine Pattern: Rename locked cade-server.exe to .old so the new one can be downloaded
        let mut server_old_exe = None;
        if is_windows && server_exe.exists() {
            let old_path = server_exe.with_extension("exe.old");
            // If an old one already exists, try to delete it first
            if old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }
            if std::fs::rename(&server_exe, &old_path).is_ok() {
                server_old_exe = Some(old_path.clone());
            }
        }

        let server_status = match update_builder_server.update_extended() {
            Ok(s) => s.updated(),
            Err(e) => {
                if let Some(old) = server_old_exe {
                    let _ = std::fs::rename(old, &server_exe);
                }
                eprintln!("\r\n[!] Warning: Failed to update cade-server (it may be running and locked).");
                eprintln!("[!] Please stop cade-server and try again. Error: {}", e);
                false
            }
        };

        Ok::<bool, crate::error::Error>(cli_status.updated() || server_status)
    })
    .await
    .map_err(|e| crate::error::Error::custom(format!("Spawn blocking error: {}", e)))??;

    Ok(status)
}

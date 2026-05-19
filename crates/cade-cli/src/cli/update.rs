use crate::Result;


pub async fn run_update(check_only: bool) -> Result<bool> {
    let status = tokio::task::spawn_blocking(move || {
        let current_exe = std::env::current_exe().map_err(|e| crate::error::Error::custom(e.to_string()))?;
        let is_windows = cfg!(windows);
        
        let cli_bin_name = if is_windows { "cade.exe" } else { "cade" };
        let server_bin_name = if is_windows { "cade-server.exe" } else { "cade-server" };
        
        let mut server_exe = current_exe.clone();
        server_exe.set_file_name(server_bin_name);

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
        let server_status = match update_builder_server.update_extended() {
            Ok(s) => s.updated(),
            Err(e) => {
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

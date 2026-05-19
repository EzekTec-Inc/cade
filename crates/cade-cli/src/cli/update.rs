use crate::Result;
use self_update::cargo_crate_version;

pub async fn run_update(check_only: bool) -> Result<bool> {
    let status = tokio::task::spawn_blocking(move || {
        let update_builder = self_update::backends::github::Update::configure()
            .repo_owner("EzekTec-Inc")
            .repo_name("cade")
            .bin_name("cade")
            .show_download_progress(true)
            .current_version(cargo_crate_version!())
            .build()
            .map_err(|e| crate::error::Error::custom(e.to_string()))?;

        if check_only {
            let latest = update_builder.get_latest_release().map_err(|e| crate::error::Error::custom(e.to_string()))?;
            let is_greater = self_update::version::bump_is_greater(cargo_crate_version!(), &latest.version).unwrap_or(false);
            Ok::<bool, crate::error::Error>(is_greater)
        } else {
            let status = update_builder.update_extended().map_err(|e| crate::error::Error::custom(e.to_string()))?;
            Ok::<bool, crate::error::Error>(status.updated())
        }
    })
    .await
    .map_err(|e| crate::error::Error::custom(format!("Spawn blocking error: {}", e)))??;

    Ok(status)
}

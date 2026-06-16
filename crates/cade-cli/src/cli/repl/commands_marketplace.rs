use super::Repl;
use crate::Result;
use dirs;

impl Repl {
    pub(crate) async fn cmd_marketplace(&self) -> Result<bool> {
        self.tui_sys("Fetching marketplace index...");
        let index_url = self.settings.lock().marketplace_url().to_string();

        let client = reqwest::Client::new();
        let resp = match client.get(index_url).send().await {
            Ok(r) => r,
            Err(e) => {
                self.tui_err(format!("Failed to connect to marketplace: {e}"));
                return Ok(false);
            }
        };

        if !resp.status().is_success() {
            self.tui_err(format!("Marketplace returned status: {}", resp.status()));
            return Ok(false);
        }

        let index: cade_plugin::marketplace::RegistryIndex = match resp.json().await {
            Ok(idx) => idx,
            Err(e) => {
                self.tui_err(format!("Failed to parse marketplace index: {e}"));
                return Ok(false);
            }
        };

        let result = self
            .marketplace_picker(self.app.clone(), &index.plugins)
            .await?;

        if let Some(crate::cli::repl::pickers::marketplace::MarketplaceActionResult::Install(
            url,
            plugin_id,
        )) = result
        {
            self.tui_sys(format!("Installing plugin {plugin_id}..."));
            let target_dir = dirs::home_dir()
                .map(|h| h.join(".cade").join("plugins"))
                .unwrap_or_default();
            match cade_plugin::marketplace::install_plugin(&url, &plugin_id, &target_dir).await {
                Ok(manifest) => {
                    self.tui_ok(format!(
                        "Successfully installed plugin '{}' (v{})",
                        manifest.name,
                        manifest.version.unwrap_or_else(|| "unknown".to_string())
                    ));
                }
                Err(e) => {
                    self.tui_err(format!("Installation failed: {e}"));
                }
            }
        }

        let _ = self.app.lock().draw();
        Ok(false)
    }
}

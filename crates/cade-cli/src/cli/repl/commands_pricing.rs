//! /pricing command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_pricing(
        &mut self,
        arg: Option<String>,
    ) -> Result<bool> {
        match arg.as_deref() {
            Some("sync") => {
                self.tui_dim("  Fetching latest pricing rules from cloud...");
                let url = "https://raw.githubusercontent.com/EzekTec-Inc/CADE/main/crates/cade-ai/src/default_pricing.json";
                match reqwest::get(url).await {
                    Ok(res) if res.status().is_success() => {
                        if let Ok(text) = res.text().await
                            && let Some(p) = dirs::home_dir()
                                .map(|h| h.join(".cade").join("pricing.json"))
                        {
                            if let Err(e) = std::fs::write(&p, text) {
                                self.tui_err(format!(
                                    "  Failed to write pricing.json: {}",
                                    e
                                ));
                            } else {
                                let mut stats =
                                    self.session_stats.lock();
                                stats.registry = std::sync::Arc::new(
                                    cade_ai::ModelRegistry::load_or_default(Some(&p)),
                                );
                                self.tui_ok("  Pricing synced successfully!");
                            }
                        }
                    }
                    _ => self.tui_err("  Failed to fetch pricing from cloud."),
                }
            }
            Some(cmd) if cmd.starts_with("set ") => {
                self.tui_err("  /pricing set is not fully implemented yet. Please edit ~/.cade/pricing.json manually.");
            }
            _ => {
                let model = self.model();
                let stats = self.session_stats.lock();
                let pricing = stats.registry.pricing_for_model(&model);
                self.tui_hdr(format!("  Pricing for model: {}", model));
                self.tui_dim(format!("  Input: ${}/1M", pricing.input));
                self.tui_dim(format!("  Output: ${}/1M", pricing.output));
                self.tui_dim(format!("  Cache Read: ${}/1M", pricing.cache_read));
                self.tui_dim(format!("  Cache Write: ${}/1M", pricing.cache_write));
                self.tui_dim("  Use /pricing sync to update from the cloud, or edit ~/.cade/pricing.json to add local overrides.");
            }
        }
        Ok(false)
    }

}

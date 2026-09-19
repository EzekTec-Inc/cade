use super::Repl;
use crate::Result;
use serde_json::json;

impl Repl {
    /// Manage the canonical PluginEngine lifecycle through the Server API.
    pub(crate) async fn cmd_plugin(&self, args: Option<String>) -> Result<bool> {
        let Some(args) = args else {
            self.tui_sys("Usage: /plugin list | install <url> <id> | uninstall <id>");
            return Ok(false);
        };
        let mut parts = args.split_whitespace();
        let Some(action) = parts.next() else {
            self.tui_sys("Usage: /plugin list | install <url> <id> | uninstall <id>");
            return Ok(false);
        };

        match action {
            "list" => match self.client.raw_get("/plugins").await {
                Ok(response) => {
                    let plugins = response["plugins"].as_array().cloned().unwrap_or_default();
                    if plugins.is_empty() {
                        self.tui_sys("No active plugins.");
                    } else {
                        for plugin in plugins {
                            let id = plugin["id"].as_str().unwrap_or("unknown");
                            let version = plugin["version"].as_str().unwrap_or("unknown");
                            let scope = plugin["scope"].as_str().unwrap_or("unknown");
                            let tools = plugin["tools_count"].as_u64().unwrap_or(0);
                            self.tui_sys(format!("{id} v{version} [{scope}] — {tools} tools"));
                        }
                    }
                }
                Err(error) => self.tui_err(format!("Plugin inventory failed: {error}")),
            },
            "install" => {
                let Some(url) = parts.next() else {
                    self.tui_err("Usage: /plugin install <url> <id>".to_string());
                    return Ok(false);
                };
                let Some(plugin_id) = parts.next() else {
                    self.tui_err("Usage: /plugin install <url> <id>".to_string());
                    return Ok(false);
                };
                self.tui_sys(format!("Installing plugin {plugin_id}..."));
                match self
                    .client
                    .raw_post(
                        "/plugins/install",
                        &json!({ "url": url, "plugin_id": plugin_id }),
                    )
                    .await
                {
                    Ok(response) => self.tui_ok(format!(
                        "Installed plugin {}",
                        response["plugin"]["name"].as_str().unwrap_or(plugin_id)
                    )),
                    Err(error) => self.tui_err(format!("Plugin installation failed: {error}")),
                }
            }
            "uninstall" => {
                let Some(plugin_id) = parts.next() else {
                    self.tui_err("Usage: /plugin uninstall <id>".to_string());
                    return Ok(false);
                };
                self.tui_sys(format!("Removing plugin {plugin_id}..."));
                match self
                    .client
                    .raw_delete(&format!("/plugins/{plugin_id}"))
                    .await
                {
                    Ok(response) => self.tui_ok(format!(
                        "Removed plugin {}",
                        response["plugin"]["name"].as_str().unwrap_or(plugin_id)
                    )),
                    Err(error) => self.tui_err(format!("Plugin removal failed: {error}")),
                }
            }
            _ => self
                .tui_err("Usage: /plugin list | install <url> <id> | uninstall <id>".to_string()),
        }

        Ok(false)
    }
}

use super::Repl;
use crate::Result;
use serde_json::json;

impl Repl {
    /// Manage the canonical PluginEngine lifecycle through the Server API.
    pub(crate) async fn cmd_plugin(&self, args: Option<String>) -> Result<bool> {
        let Some(args) = args else {
            self.tui_sys("Usage: /plugin list | reload | search <query> | install-market <id> | install <url> <id> | uninstall <id>");
            return Ok(false);
        };
        let mut parts = args.split_whitespace();
        let Some(action) = parts.next() else {
            self.tui_sys("Usage: /plugin list | reload | search <query> | install-market <id> | install <url> <id> | uninstall <id>");
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
            "search" => {
                let query = parts.collect::<Vec<_>>().join(" ");
                self.tui_sys(if query.is_empty() {
                    "Searching marketplace catalog for all plugins...".to_string()
                } else {
                    format!("Searching marketplace catalog for '{query}'...")
                });
                let endpoint = if query.is_empty() {
                    "/plugins/search".to_string()
                } else {
                    format!("/plugins/search?query={}", query.trim())
                };
                match self.client.raw_get(&endpoint).await {
                    Ok(response) => {
                        let plugins = response["plugins"].as_array().cloned().unwrap_or_default();
                        if plugins.is_empty() {
                            self.tui_sys("No matching plugins found in marketplace.");
                        } else {
                            self.tui_ok(format!("Found {} marketplace plugin(s):", plugins.len()));
                            for p in plugins {
                                let id = p["id"].as_str().unwrap_or("unknown");
                                let version = p["version"].as_str().unwrap_or("0.1.0");
                                let desc = p["description"].as_str().unwrap_or("");
                                let author = p["author"].as_str().unwrap_or("");
                                self.tui_sys(format!("  • {id} v{version} by {author} — {desc}"));
                            }
                        }
                    }
                    Err(error) => self.tui_err(format!("Marketplace search failed: {error}")),
                }
            }
            "install-market" => {
                let Some(plugin_id) = parts.next() else {
                    self.tui_err("Usage: /plugin install-market <plugin_id>".to_string());
                    return Ok(false);
                };
                self.tui_sys(format!("Installing {plugin_id} from marketplace..."));
                match self
                    .client
                    .raw_post(
                        "/plugins/install",
                        &json!({ "url": plugin_id, "plugin_id": plugin_id }),
                    )
                    .await
                {
                    Ok(response) => self.tui_ok(format!(
                        "Installed marketplace plugin {}",
                        response["plugin"]["name"].as_str().unwrap_or(plugin_id)
                    )),
                    Err(error) => self.tui_err(format!("Marketplace installation failed: {error}")),
                }
            }
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
            "reload" => {
                self.tui_sys("Hot-swapping and reloading plugin registry...".to_string());
                match self.client.raw_post("/plugins/reload", &json!({})).await {
                    Ok(response) => {
                        let count = response["plugins_count"].as_u64().unwrap_or(0);
                        let tools = response["tools_count"].as_u64().unwrap_or(0);
                        self.tui_ok(format!(
                            "Reloaded {count} active plugin(s) ({tools} tools ready)."
                        ));
                    }
                    Err(error) => self.tui_err(format!("Plugin reload failed: {error}")),
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
            _ => self.tui_err(
                "Usage: /plugin list | reload | search <query> | install-market <id> | install <url> <id> | uninstall <id>"
                    .to_string(),
            ),
        }

        Ok(false)
    }
}

//! /backend command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_backend(&mut self, backend_arg: Option<String>) -> Result<bool> {
        let current = self.exec_backend.name();
        match backend_arg {
            None => {
                self.tui_hdr(format!("  Execution backend: {current}"));
                self.tui_dim("  Available: local, docker, ssh, readonly, virtual".to_string());
                self.tui_dim("  Change: /backend local|docker|ssh|readonly|virtual".to_string());
                self.tui_dim("  Or set in ~/.cade/settings.json: { \"execution\": { \"backend\": \"virtual\" } }".to_string());
            }
            Some(new_backend) => {
                use cade_core::settings::ExecutionBackendKind;
                match new_backend.parse::<ExecutionBackendKind>() {
                    Err(e) => self.tui_err(format!("  ✗ {e}")),
                    Ok(kind) => {
                        // Build a new backend from the current settings profile
                        // with the backend kind overridden
                        let profile = {
                            let s = self.settings.lock();
                            let mut p = s.execution_profile().clone();
                            p.backend = kind;
                            p
                        };
                        let new_b = cade_agent::backends::backend_from_profile(&profile);
                        let name = new_b.name();
                        self.exec_backend = std::sync::Arc::from(new_b);
                        self.tui_ok(format!("  ✓ Switched to {name} backend"));
                        if name == "docker" {
                            let docker_image =
                                profile.docker_image.as_deref().unwrap_or("ubuntu:22.04");
                            self.tui_dim(format!("  Image: {docker_image}  (set execution.docker_image in settings to change)"));
                        } else if name == "ssh" {
                            let host = profile.ssh_host.as_deref().unwrap_or("(not configured)");
                            self.tui_dim(format!(
                                "  Host: {host}  (set execution.ssh_host in settings)"
                            ));
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

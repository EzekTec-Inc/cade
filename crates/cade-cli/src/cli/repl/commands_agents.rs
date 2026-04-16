//! /agents command handler.

use crate::Result;
use super::{AgentPickerResult, Repl};
use std::sync::Arc;

impl Repl {
    pub(crate) async fn cmd_agents(
        &mut self,
    ) -> Result<bool> {
            if self.require_capability(
                cade_core::capabilities::Capability::Agentic,
                "/agents",
            ) {
                return Ok(false);
            }
            self.tui_dim("  Fetching agents…");
            match self.client.list_agents().await {
                Ok(agents) if agents.is_empty() => {
                    self.tui_dim("  (no agents found)");
                }
                Ok(mut agents) => {
                    if let Some(result) = self
                        .agent_picker(Arc::clone(&self.app), &mut agents)
                        .await?
                    {
                        match result {
                            AgentPickerResult::Switch(a) => {
                                *self.agent_id.lock() =
                                    a.id.clone();
                                *self.agent_name.lock() =
                                    a.name.clone();
                                { let mut s = self.settings.lock();
                                    let _ = s.set_last_agent(&a.id);
                                }
                                { let mut s = self.session.lock();
                                    let _ = s.set_agent(a.id.clone(), Some(a.name.clone()));
                                }
                                self.tui_ok(format!(
                                    "  ✓ Switched to: {} ({})",
                                    a.name, a.id
                                ));
                            }
                            AgentPickerResult::Rename { agent, new_name } => match self
                                .client
                                .rename_agent(&agent.id, &new_name)
                                .await
                            {
                                Ok(_) => {
                                    if agent.id == self.agent_id() {
                                        *self
                                            .agent_name
                                            .lock() = new_name.clone();
                                    }
                                    self.tui_ok(format!(
                                        "  ✓ Renamed '{}' → '{new_name}'",
                                        agent.name
                                    ));
                                }
                                Err(e) => self.tui_err(e.to_string()),
                            },
                            AgentPickerResult::DeleteMany(to_delete) => {
                                let current_id = self.agent_id();
                                let mut deleted_active = false;
                                for a in &to_delete {
                                    match self.client.delete_agent(&a.id).await {
                                        Ok(_) => {
                                            self.tui_ok(format!(
                                                "  ✓ Deleted: {}",
                                                a.name
                                            ));
                                            if a.id == current_id {
                                                deleted_active = true;
                                            }
                                        }
                                        Err(e) => self.tui_err(e.to_string()),
                                    }
                                }
                                if deleted_active {
                                    match self.client.list_agents().await {
                                        Ok(remaining) if !remaining.is_empty() => {
                                            let first = &remaining[0];
                                            *self
                                                .agent_id
                                                .lock() =
                                                first.id.clone();
                                            *self
                                                .agent_name
                                                .lock() =
                                                first.name.clone();
                                            { let mut s = self.settings.lock();
                                                let _ = s.set_last_agent(&first.id);
                                            }
                                            { let mut s = self.session.lock();
                                                let _ = s.set_agent(first.id.clone(), Some(first.name.clone()));
                                            }
                                            self.tui_dim(format!(
                                                "  → Now using: {}",
                                                first.name
                                            ));
                                        }
                                        _ => {
                                            self.tui_dim("  No remaining agents — run /new to create one");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = self.app.lock().draw();
                }
                Err(e) => self.tui_err(e.to_string()),
            }
        Ok(false)
    }
}

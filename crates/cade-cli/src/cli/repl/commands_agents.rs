//! /agents command handler.

use super::{AgentPickerResult, Repl};
use crate::Result;
use std::sync::Arc;

impl Repl {
    fn command_session(&self) -> super::command_lifecycle::CommandSession<'_> {
        super::command_lifecycle::CommandSession {
            agent_id: &self.agent_id,
            agent_name: &self.agent_name,
            conversation_id: &self.conversation_id,
            store: &self.session,
        }
    }

    pub(crate) fn select_conversation(&self, conversation: Option<String>) -> Result<()> {
        self.app.lock().reset_context();
        self.command_session().select_conversation(conversation)
    }

    pub(crate) fn select_agent(
        &mut self,
        agent: &cade_agent::agent::client::AgentState,
    ) -> Result<()> {
        let changed = self.agent_id() != agent.id;
        self.command_session().select_agent(agent)?;
        if changed {
            self.first_turn
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.last_assistant_text.lock().clear();
            self.last_reasoning.lock().clear();
            self.write_tool_calls
                .store(0, std::sync::atomic::Ordering::SeqCst);
            self.writes_at_last_active_goal_update
                .store(0, std::sync::atomic::Ordering::SeqCst);
            if let Some(model) = agent.model.as_ref().filter(|model| !model.is_empty()) {
                *self.current_model.lock() = model.clone();
            }
            let mut app = self.app.lock();
            let _ = app.clear_content();
            app.reset_context();
            app.update_agent_name(agent.name.clone());
            app.update_model(self.model());
        }
        // Working Session and its grants deliberately survive selection.
        let saved = self.settings.lock().set_last_agent(&agent.id);
        if let Err(error) = saved {
            self.tui_err(format!(
                "Agent selected, but could not save last-agent preference: {error}"
            ));
        }
        Ok(())
    }

    pub(crate) async fn cmd_agents(&mut self) -> Result<bool> {
        if self.require_capability(cade_core::capabilities::Capability::Agentic, "/agents") {
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
                            if let Err(error) = self.select_agent(&a) {
                                self.tui_err(format!("Could not switch agent: {error}"));
                                return Ok(false);
                            }
                            self.tui_ok(format!("  ✓ Switched to: {} ({})", a.name, a.id));
                        }
                        AgentPickerResult::Rename { agent, new_name } => match self
                            .client
                            .rename_agent(&agent.id, &new_name)
                            .await
                        {
                            Ok(_) => {
                                if agent.id == self.agent_id() {
                                    *self.agent_name.lock() = new_name.clone();
                                }
                                self.tui_ok(format!("  ✓ Renamed '{}' → '{new_name}'", agent.name));
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        },
                        AgentPickerResult::DeleteMany(to_delete) => {
                            let current_id = self.agent_id();
                            let mut deleted_active = false;
                            for a in &to_delete {
                                match self.client.delete_agent(&a.id).await {
                                    Ok(_) => {
                                        self.tui_ok(format!("  ✓ Deleted: {}", a.name));
                                        if a.id == current_id {
                                            deleted_active = true;
                                            if let Err(error) = self.select_conversation(None) {
                                                self.tui_err(format!("Could not clear deleted agent conversation: {error}"));
                                            }
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            if deleted_active {
                                match self.client.list_agents().await {
                                    Ok(remaining) if !remaining.is_empty() => {
                                        let first = &remaining[0];
                                        if let Err(error) = self.select_agent(first) {
                                            self.tui_err(format!("Could not select remaining agent: {error}"));
                                            return Ok(false);
                                        }
                                        self.tui_dim(format!("  → Now using: {}", first.name));
                                    }
                                    Ok(_) => {
                                        self.tui_dim(
                                            "  No remaining agents — run /new-agent to create one",
                                        );
                                    }
                                    Err(error) => self.tui_err(format!("Could not load remaining agents: {error}. Use /agents or /new-agent.")),
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

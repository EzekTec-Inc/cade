//! /newagent command handler.

use super::Repl;
use crate::Result;
use crate::ui::RenderLine;

impl Repl {
    pub(crate) async fn cmd_newagent(
        &mut self,
        _pending_input: &mut Option<String>,
    ) -> Result<bool> {
        let _ = self
            .app
            .lock()
            .push(RenderLine::SystemMsg("  Creating new agent…".to_string()));
        // S5: Offer to copy `human` and `project` blocks from current agent
        let prev_agent_id = self.agent_id();
        let inherit_blocks: Vec<(String, String, String)> = {
            let blocks = self
                .client
                .get_memory(&prev_agent_id)
                .await
                .unwrap_or_default();
            blocks
                .into_iter()
                .filter(|b| {
                    (b.label == "human" || b.label == "project") && !b.value.trim().is_empty()
                })
                .map(|b| {
                    (
                        b.label.clone(),
                        b.value.clone(),
                        b.description.clone().unwrap_or_default(),
                    )
                })
                .collect()
        };
        let copy_memory = if !inherit_blocks.is_empty() {
            let summary: String = inherit_blocks
                .iter()
                .map(|(l, v, _)| format!("{} ({} chars)", l, v.chars().count()))
                .collect::<Vec<_>>()
                .join(", ");
            let q = crate::ui::question::Question {
                header: "Copy memory".to_string(),
                text: format!("Copy memory to new agent? ({summary})"),
                options: vec![
                    crate::ui::question::QuestionOption {
                        label: "Yes — copy human + project blocks".to_string(),
                        description: "Start new agent with existing context".to_string(),
                    },
                    crate::ui::question::QuestionOption {
                        label: "No — start fresh".to_string(),
                        description: "New agent gets empty memory blocks".to_string(),
                    },
                ],
                multi_select: false,
                allow_other: false,
                progress: None,
            };
            let ans = {
                let mut app = self.app.lock();
                let r = app.ask_question(&q);
                app.scroll = 0;
                let _ = app.draw();
                r
            };
            matches!(&ans, Ok(Some(a)) if a.as_str().starts_with("Yes"))
        } else {
            false
        };
        let model = self.model();
        let req = cade_agent::agent::client::CreateAgentRequest {
            name: Some(format!(
                "CADE-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            )),
            model,
            description: Some("CADE coding agent".to_string()),
            system_prompt: None,
            memory_blocks: vec![],
            tool_ids: vec![],
        };
        match self.client.create_agent(req).await {
            Ok(a) => {
                *self.agent_id.lock() = a.id.clone();
                *self.agent_name.lock() = a.name.clone();
                *self.conversation_id.lock() = None;
                {
                    let mut s = self.settings.lock();
                    let _ = s.set_last_agent(&a.id);
                }
                {
                    let mut s = self.session.lock();
                    let _ = s.set_agent(a.id.clone(), Some(a.name.clone()));
                }
                let _ = self.app.lock().push(RenderLine::SystemMsg(format!(
                    "  ✓ New agent: {} ({})",
                    a.name, a.id
                )));
                // S5: copy inherited blocks to new agent
                if copy_memory {
                    for (label, value, desc) in &inherit_blocks {
                        let desc_opt = if desc.is_empty() {
                            None
                        } else {
                            Some(desc.as_str())
                        };
                        let _ = self
                            .client
                            .upsert_memory(&a.id, label, value, desc_opt)
                            .await;
                    }
                    let n = inherit_blocks.len();
                    let _ = self.app.lock().push(RenderLine::SystemMsg(format!(
                        "  ✓ Copied {n} memory block(s) from previous agent"
                    )));
                }
                self.spawn_tool_reregister();
            }
            Err(e) => self.tui_err(e.to_string()),
        }
        Ok(false)
    }
}

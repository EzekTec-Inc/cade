//! /delete command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_delete(
        &mut self,
        target: Option<String>,
        stdout: &mut std::io::Stdout,
        _pending_input: &mut Option<String>,
    ) -> Result<bool> {
        // /delete [name-or-id] — delete a specific agent by name/id prefix
        let agents = match self.client.list_agents().await {
            Ok(a) => a,
            Err(e) => {
                self.print_error(stdout, &e.to_string())?;
                vec![]
            }
        };
        if agents.is_empty() {
            self.tui_dim("  (no agents)");
        } else if let Some(query) = target {
            let q = query.to_lowercase();
            let matched: Vec<_> = agents
                .iter()
                .filter(|a| a.name.to_lowercase().contains(&q) || a.id.starts_with(&q))
                .collect();
            match matched.len() {
                0 => self.tui_err(format!("No agent matching '{query}'")),
                1 => {
                    let a = matched[0];
                    use crate::ui::question::{Question, QuestionOption};
                    let opts = vec![
                        QuestionOption {
                            label: "Yes — delete".to_string(),
                            description: String::new(),
                        },
                        QuestionOption {
                            label: "No — cancel".to_string(),
                            description: String::new(),
                        },
                    ];
                    let q_widget = Question {
                        header: "Confirm delete".to_string(),
                        text: format!("Delete '{}'?", a.name),
                        options: opts.clone(),
                        multi_select: false,
                        allow_other: false,
                        progress: None,
                    };
                    let confirmed = {
                        let mut app = self.app.lock();
                        let r = app.ask_question(&q_widget)?;
                        app.scroll = 0;
                        let _ = app.draw();
                        matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                    };
                    if confirmed {
                        match self.client.delete_agent(&a.id).await {
                            Ok(_) => {
                                self.tui_ok(format!("  ✓ Deleted: {}", a.name));
                                if a.id == self.agent_id() {
                                    self.tui_dim(
                                        "  Active agent deleted — use /new or /agents to continue",
                                    );
                                }
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    } else {
                        self.tui_dim("  (cancelled)");
                    }
                }
                n => self.tui_err(format!("{n} agents match '{query}' — be more specific")),
            }
        } else {
            self.tui_dim("  Usage: /delete <name-or-id>  or  /agents then press d");
        }
        Ok(false)
    }
}

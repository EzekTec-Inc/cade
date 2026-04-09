//! /memory command handler.

use crate::Result;
use crate::ui::{RenderLine, ToastLevel};
use super::{MemoryPickerResult, Repl};

impl Repl {
    /// Handle all `/memory` subcommands.
    /// Returns `Ok(false)` (never exits the REPL).
    pub(crate) async fn cmd_memory(
        &mut self,
        input: &str,
        _stdout: &mut std::io::Stdout,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
            // Parse subcommand from the raw input line
            let raw = input.trim();
            let mem_arg = raw.strip_prefix("/memory").unwrap_or("").trim().to_string();
            let parts: Vec<&str> = mem_arg.splitn(4, ' ').collect();
            let sub = parts.first().copied().unwrap_or("");

            match sub {
                // /memory view <label> — show full value untruncated
                "view" | "show" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.get_memory(&id).await {
                        Ok(blocks) => {
                            if let Some(b) = blocks.iter().find(|b| b.label == label) {
                                self.tui_blank();
                                self.tui_hdr(format!("  [{label}]"));
                                if let Some(desc) = &b.description
                                    && !desc.is_empty()
                                {
                                    self.tui_dim(format!("  {desc}"));
                                }
                                self.tui_blank();
                                if b.value.is_empty() {
                                    self.tui_dim("  (empty)");
                                } else {
                                    for ln in b.value.lines() {
                                        self.tui_sys(ln.to_string());
                                    }
                                }
                            } else {
                                self.tui_err(format!("Block '{label}' not found"));
                            }
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory set <label> <value>
                "set" if parts.len() >= 3 => {
                    let label = parts[1];
                    let value = parts[2..].join(" ");
                    let id = self.agent_id();
                    match self.client.upsert_memory(&id, label, &value, None).await {
                        Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory delete <label>
                "delete" | "del" | "rm" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.delete_memory(&id, label).await {
                        Ok(_) => self.tui_ok(format!("  ✓ [{label}] deleted")),
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory edit <label> — inline multi-line editor via QuestionWidget
                "edit" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    let current = self
                        .client
                        .get_memory(&id)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .find(|b| b.label == label)
                        .map(|b| b.value)
                        .unwrap_or_default();
                    use crate::ui::question::{Question, QuestionOption};
                    let opts = vec![
                        QuestionOption {
                            label: format!(
                                "Keep: {}…",
                                current.chars().take(60).collect::<String>()
                            ),
                            description: String::new(),
                        },
                        QuestionOption {
                            label: "Clear (erase block)".to_string(),
                            description: String::new(),
                        },
                    ];
                    let q = Question {
                        header: "Edit memory".to_string(),
                        text: format!("Type new value for [{label}] or pick action:"),
                        options: opts.clone(),
                        multi_select: false,
                        allow_other: true,
                        progress: None,
                    };
                    let ans = {
                        let mut app = self.app.lock();
                        app.ask_question(&q)?
                    };
                    if let Some(a) = &ans {
                        let val = a.as_str();
                        let new_value = if val.starts_with("Clear") {
                            String::new()
                        } else if val.starts_with("Keep") {
                            current
                        } else {
                            val.to_string()
                        };
                        match self
                            .client
                            .upsert_memory(&id, label, &new_value, None)
                            .await
                        {
                            Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }
                }
                // /memory history <label> — show last 5 revisions
                "history" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.list_memory_history(&id, label, 5).await {
                        Ok(revs) if revs.is_empty() => {
                            let _ = self.app.lock().push(
                                RenderLine::SystemMsg(format!(
                                    "  [{label}] no history recorded yet"
                                )),
                            );
                        }
                        Ok(revs) => {
                            let _ = self
                                .app
                                .lock()
                                .push(RenderLine::Blank);
                            for (i, rev) in revs.iter().enumerate() {
                                let rev_id = rev["id"].as_str().unwrap_or("");
                                let ts = rev["updated_at"].as_i64().unwrap_or(0);
                                let val = rev["value"].as_str().unwrap_or("");
                                let preview: String = val.chars().take(120).collect();
                                let ellipsis = if val.len() > 120 { "…" } else { "" };
                                let _ = self.app.lock().push(
                                    RenderLine::SystemMsg(format!(
                                        "  [{i}] {ts}  id={rev_id}"
                                    )),
                                );
                                let _ = self.app.lock().push(
                                    RenderLine::SystemMsg(format!(
                                        "      {preview}{ellipsis}"
                                    )),
                                );
                                let _ = self
                                    .app
                                    .lock()
                                    .push(RenderLine::Blank);
                            }
                            let _ = self.app.lock().push(
                                RenderLine::SystemMsg(format!(
                                    "  Use: /memory restore {label} <id>"
                                )),
                            );
                        }
                        Err(e) => {
                            let _ = self
                                .app
                                .lock()
                                .push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                        }
                    }
                }
                // /memory restore <label> <rev_id>
                "restore" if parts.len() >= 3 => {
                    let label = parts[1];
                    let rev_id = parts[2];
                    let id = self.agent_id();
                    match self.client.restore_memory(&id, label, rev_id).await {
                        Ok(_) => {
                            let _ = self.app.lock().push(
                                RenderLine::SystemMsg(format!(
                                    "  ✓ [{label}] restored to revision {rev_id}"
                                )),
                            );
                        }
                        Err(e) => {
                            let _ = self
                                .app
                                .lock()
                                .push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                        }
                    }
                }
                // /memory pin <label>
                "pin" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.pin_memory(&id, label).await {
                        Ok(_) => self
                            .tui_ok(format!("  📌 [{label}] pinned — always injected")),
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory unpin <label>
                "unpin" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.promote_memory(&id, label).await {
                        Ok(_) => {
                            self.tui_ok(format!("  ● [{label}] unpinned → short-term"))
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory promote <label> — reactivate archived block
                "promote" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.promote_memory(&id, label).await {
                        Ok(_) => {
                            self.tui_ok(format!("  ● [{label}] promoted → short-term"))
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
                // /memory demote <label> — manually archive block
                "demote" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    match self.client.demote_memory(&id, label).await {
                        Ok(_) => self.tui_ok(format!(
                            "  ○ [{label}] demoted → long-term (archived)"
                        )),
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }

                // /memory why <label> — show provenance chain
                "why" if parts.len() >= 2 => {
                    let label = parts[1];
                    let id = self.agent_id();
                    self.tui_dim(format!("  Looking up provenance for '{label}'…"));
                    match self.client.get_memory_why(&id, label).await {
                        Ok(summary) => {
                            self.tui_blank();
                            for line in summary.lines() {
                                self.tui_sys(format!("  {line}"));
                            }
                        }
                        Err(e) => self.tui_err(format!("  ✗ {e}")),
                    }
                }

                // /memory typed [type] — filter blocks by memory_type
                "typed" => {
                    let filter = parts.get(1).copied();
                    let id = self.agent_id();
                    match self.client.get_memory(&id).await {
                        Ok(blocks) => {
                            let label = filter.unwrap_or("all");
                            self.tui_hdr(format!("  Memory blocks (type={label}):"));
                            let mut shown = 0;
                            for b in &blocks {
                                // Only blocks with a type label match (server doesn't
                                // return memory_type yet; shown inline via describe)
                                shown += 1;
                                self.tui_dim(format!(
                                    "  [{badge}]  {label}",
                                    badge = b.tier.as_deref().unwrap_or("short"),
                                    label = b.label,
                                ));
                            }
                            if shown == 0 {
                                self.tui_dim("  (none)".to_string());
                            }
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }

                // /memory audit — find stale / low-confidence blocks
                "audit" => {
                    let id = self.agent_id();
                    match self.client.get_memory(&id).await {
                        Ok(blocks) => {
                            let empty_blocks: Vec<_> = blocks
                                .iter()
                                .filter(|b| b.value.trim().is_empty())
                                .collect();
                            let long_blocks: Vec<_> = blocks
                                .iter()
                                .filter(|b| b.tier.as_deref() == Some("long"))
                                .collect();
                            self.tui_hdr(format!(
                                "  Memory audit — {} total blocks:",
                                blocks.len()
                            ));
                            if !empty_blocks.is_empty() {
                                self.tui_dim(format!(
                                    "  ⚠  {} empty block(s): {}",
                                    empty_blocks.len(),
                                    empty_blocks
                                        .iter()
                                        .map(|b| b.label.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ));
                            }
                            if !long_blocks.is_empty() {
                                self.tui_dim(format!(
                                    "  ○  {} archived block(s): {}",
                                    long_blocks.len(),
                                    long_blocks
                                        .iter()
                                        .map(|b| b.label.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ));
                            }
                            if empty_blocks.is_empty() && long_blocks.is_empty() {
                                self.tui_ok(
                                    "  ✓ All blocks active and populated.".to_string(),
                                );
                            }
                            self.tui_dim("  Use /reflect to trigger automatic extraction from conversation.".to_string());
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }

                // /memory suggest — run lightweight reflection
                "suggest" => {
                    let id = self.agent_id();
                    self.tui_dim("  Triggering reflection…".to_string());
                    match self.client.trigger_reflect(&id, None).await {
                        Ok(summary) => self.tui_ok(format!("  {summary}")),
                        Err(e) => self.tui_err(format!("  ✗ {e}")),
                    }
                }

                // /memory (list)
                _ => {
                    let id = self.agent_id();
                    match self.client.get_memory(&id).await {
                        Ok(mut blocks) => {
                            match self
                                .memory_picker(
                                    std::sync::Arc::clone(&self.app),
                                    &mut blocks,
                                )
                                .await
                            {
                                Ok(Some(MemoryPickerResult::Edit(b))) => {
                                    *pending_input =
                                        Some(format!("/memory edit {}", b.label));
                                }
                                Ok(Some(MemoryPickerResult::Delete(b))) => {
                                    *pending_input =
                                        Some(format!("/memory delete {}", b.label));
                                }
                                Ok(Some(MemoryPickerResult::TogglePin(b))) => {
                                    let is_pinned =
                                        b.tier.as_deref() == Some("pinned");
                                    let cmd =
                                        if is_pinned { "unpin" } else { "pin" };
                                    *pending_input =
                                        Some(format!("/memory {cmd} {}", b.label));
                                }
                                Ok(None) => {} // cancelled
                                Err(e) => {
                                    self.tui_err(e.to_string());
                                }
                            }
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
            }
        Ok(false)
    }

    pub(crate) async fn cmd_reflect(
        &mut self,
        focus_arg: Option<String>,
    ) -> Result<bool> {
            if self.require_capability(
                cade_core::capabilities::Capability::Agentic,
                "/reflect",
            ) {
                return Ok(false);
            }
            let agent_id = self.agent_id();
            let focus = focus_arg.as_deref();
            let focus_msg = focus.map(|f| format!(" (focus: {f})")).unwrap_or_default();
            self.tui_dim(format!("  Reflecting on conversation history{focus_msg}…"));
            match self.client.trigger_reflect(&agent_id, focus).await {
                Ok(summary) => self.tui_ok(format!("  ✓ {summary}")),
                Err(e) => self.tui_err(format!("  ✗ Reflect failed: {e}")),
            }
        Ok(false)
    }

    pub(crate) async fn cmd_remember(
        &mut self,
        text: String,
    ) -> Result<bool> {
            // Route through the agent — it decides what to store and where.
            // This matches CADE's /remember behaviour exactly.
            let msg = if text.is_empty() {
                "[/remember] Please review our recent conversation and update your \
                 memory blocks with anything important you've learned about me, \
                 my preferences, or this project."
                    .to_string()
            } else {
                format!("[/remember] {text}")
            };
            let mut stdout = std::io::stdout();
            self.agent_turn(&mut stdout, &msg).await?;
            let _ = self.app.lock().commit_streaming();
        Ok(false)
    }

    pub(crate) async fn cmd_pin(
        &mut self,
    ) -> Result<bool> {
            let id = self.agent_id();
            let name = self.agent_name();
            { let mut s = self.settings.lock();
                match s.pin_agent(&id, &name) {
                    Ok(_) => {
                        self.app.lock().show_toast(
                            format!("Pinned agent: {name}"),
                            ToastLevel::Success,
                        );
                        self.tui_ok(format!("  ✓ Pinned: {name} ({id})"));
                    }
                    Err(e) => self.tui_err(format!("Pin failed: {e}")),
                }
            }
        Ok(false)
    }
}

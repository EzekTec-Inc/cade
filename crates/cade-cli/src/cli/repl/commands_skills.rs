//! /skills command handler.

use super::Repl;
use crate::Result;
use crate::ui::RenderLine;

impl Repl {
    /// Handle all `/skills` subcommands.
    /// Returns `Ok(false)` (never exits the REPL).
    pub(crate) async fn cmd_skills(
        &mut self,
        arg: Option<String>,
        stdout: &mut std::io::Stdout,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
        {
            let sub = arg.as_deref().unwrap_or("list");
            let (sub_cmd, sub_arg) = sub
                .splitn(2, ' ')
                .collect::<Vec<_>>()
                .split_first()
                .map(|(c, r)| (*c, r.join(" ")))
                .unwrap_or(("list", String::new()));

            match sub_cmd {
                "list" | "" => {
                    let skills = self.skills.lock();
                    let agent_id = self.agent_id();
                    if skills.is_empty() {
                        let mut app = self.app.lock();
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::InfoHeader(
                            "  ◆ Skills  (none loaded)".to_string(),
                        ));
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::DimMsg(
                            "  No skills found. Searched:".to_string(),
                        ));
                        let _ = app.push(RenderLine::Pair {
                            label: "project".to_string(),
                            value: ".cade/skills/".to_string(),
                        });
                        let _ = app.push(RenderLine::Pair {
                            label: "global".to_string(),
                            value: "~/.cade/skills/".to_string(),
                        });
                        let _ = app.push(RenderLine::Pair {
                            label: "agent".to_string(),
                            value: format!("~/.cade/subagents/{agent_id}/skills/"),
                        });
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::DimMsg(
                            "  /skills new <name>  to scaffold your first skill".to_string(),
                        ));
                        let _ = app.push(RenderLine::Blank);
                    } else {
                        let mut sorted: Vec<_> = skills.iter().cloned().collect();
                        sorted.sort_by(|a, b| {
                            a.scope
                                .display_order()
                                .cmp(&b.scope.display_order())
                                .then(a.id.cmp(&b.id))
                        });
                        drop(skills);

                        let chosen = {
                            let mut app = self.app.lock();
                            let colors = app.colors.clone();
                            crate::ui::skills::show_skills_manager(
                                &mut app.terminal,
                                sorted,
                                &colors,
                            )?
                        };
                        let _ = self.app.lock().draw();

                        if let Some(crate::ui::skills::SkillsAction::Reload) = chosen {
                            *pending_input = Some("/skills reload".to_string());
                        }
                    }
                }

                "create" | "new" => {
                    let name_raw = sub_arg.trim().to_string();
                    if name_raw.is_empty() {
                        self.tui_dim("  Usage: /skills create <name>");
                    } else {
                        let slug: String = name_raw
                            .to_lowercase()
                            .chars()
                            .map(|c| if c.is_alphanumeric() { c } else { '-' })
                            .collect::<String>()
                            .trim_matches('-')
                            .to_string();
                        let skill_dir = self.skills_dir.join(&slug);
                        let skill_file = skill_dir.join("SKILL.MD");
                        if skill_file.exists() {
                            self.tui_err(format!(
                                "Skill '{}' already exists: {}",
                                slug,
                                skill_file.display()
                            ));
                        } else {
                            match std::fs::create_dir_all(&skill_dir) {
                                Ok(_) => {
                                    let title: String = slug
                                        .replace('-', " ")
                                        .split_whitespace()
                                        .map(|w| {
                                            let mut c = w.chars();
                                            match c.next() {
                                                None => String::new(),
                                                Some(f) => {
                                                    f.to_uppercase().collect::<String>()
                                                        + c.as_str()
                                                }
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    let template = format!(
                                        "---\nname: {title}\ndescription: One-line description of what this skill does\ncategory: general\ntags: []\n---\n\n\
                                            # {title}\n\nDescribe the skill here. This text is injected into the agent's\n\
                                            system prompt when this skill is loaded.\n\n\
                                            You can use markdown, code blocks, examples, step-by-step instructions, etc.\n"
                                    );
                                    match std::fs::write(&skill_file, template) {
                                        Ok(_) => {
                                            self.tui_ok(format!(
                                                "  ✓ Created: {}",
                                                skill_file.display()
                                            ));
                                            self.tui_dim(format!("  /skills reload  to activate  ·  open {} in your editor to customize", skill_file.display()));
                                        }
                                        Err(e) => {
                                            self.tui_err(format!("Failed to write skill file: {e}"))
                                        }
                                    }
                                }
                                Err(e) => self.tui_err(format!("Failed to create directory: {e}")),
                            }
                        }
                    }
                }

                "reload" => {
                    let agent_id = self.agent_id();
                    let new_skills =
                        cade_core::skills::discover_all_skills(&self.cwd, Some(&agent_id), None);
                    let prev_count = self.skills.lock().len();
                    let new_count = new_skills.len();

                    let existing = self.client.get_memory(&agent_id).await.unwrap_or_default();
                    for block in &existing {
                        if block.label.starts_with("skill:") {
                            let _ = self.client.delete_memory(&agent_id, &block.label).await;
                        }
                    }
                    let mut names = vec![];
                    for skill in &new_skills {
                        let label = format!("skill:{}", skill.id);
                        let _ = self
                            .client
                            .upsert_memory(&agent_id, &label, &skill.to_context_block(), None)
                            .await;
                        names.push(skill.name.clone());
                    }

                    let listing = cade_core::skills::skills_listing(&new_skills);
                    let _ = self
                        .client
                        .upsert_memory(&agent_id, "skills", listing.as_deref().unwrap_or(""), None)
                        .await;

                    *self.skills.lock() = new_skills;

                    self.tui_ok(format!(
                        "  ✓ Skills reloaded  ({new_count} loaded, was {prev_count})"
                    ));

                    if new_count > 0 {
                        let list = names.join(", ");
                        let notify = format!(
                            "[System: Skills reloaded. Now active: {list}. \
                                         Use load_skill(id) to load any skill's full content.]"
                        );
                        self.agent_turn(stdout, &notify).await?;
                        let _ = self.app.lock().commit_streaming();
                    }
                }

                "delete" | "rm" => {
                    let id = sub_arg.trim();
                    if id.is_empty() {
                        self.tui_err("  Usage: /skills delete <id>");
                    } else {
                        let skill_dir = self.skills_dir.join(id);
                        if !skill_dir.exists() {
                            self.tui_err(format!(
                                "  Skill directory not found: {}",
                                skill_dir.display()
                            ));
                            self.tui_dim("  Run /skills to list available skills.");
                        } else {
                            self.tui_sys(format!(
                                "  Deleting skill '{id}' at: {}",
                                skill_dir.display()
                            ));
                            match std::fs::remove_dir_all(&skill_dir) {
                                Ok(_) => {
                                    // Remove from in-memory list
                                    self.skills.lock().retain(|s| s.id != id);
                                    // Update memory
                                    let agent_id = self.agent_id();
                                    let skills_snap = self.skills.lock().clone();
                                    let listing = cade_core::skills::skills_listing(&skills_snap);
                                    let _ = self
                                        .client
                                        .upsert_memory(
                                            &agent_id,
                                            "skills",
                                            listing.as_deref().unwrap_or(""),
                                            None,
                                        )
                                        .await;
                                    let _ = self
                                        .client
                                        .delete_memory(&agent_id, &format!("skill:{id}"))
                                        .await;
                                    self.tui_ok(format!("  ✓ Deleted skill '{id}'"));
                                    self.tui_dim("  /skills reload  to update agent context");
                                }
                                Err(e) => self.tui_err(format!("  Failed to delete: {e}")),
                            }
                        }
                    }
                }

                "disable" => {
                    let id = sub_arg.trim();
                    if id.is_empty() {
                        self.tui_err("  Usage: /skills disable <id>");
                    } else {
                        let agent_id = self.agent_id();
                        match self.client.disable_skill_on_server(&agent_id, id).await {
                            Ok(()) => {
                                self.tui_ok(format!(
                                        "  ✓ Skill '{id}' disabled — will be excluded from context on next turn."
                                    ));
                                self.tui_dim("  /skills enable <id>  to re-enable");
                            }
                            Err(e) => self.tui_err(format!("  Failed to disable skill: {e}")),
                        }
                    }
                }

                "enable" => {
                    let id = sub_arg.trim();
                    if id.is_empty() {
                        self.tui_err("  Usage: /skills enable <id>");
                    } else {
                        let agent_id = self.agent_id();
                        match self.client.enable_skill_on_server(&agent_id, id).await {
                            Ok(()) => {
                                self.tui_ok(format!(
                                        "  ✓ Skill '{id}' re-enabled — will be included in context on next turn."
                                    ));
                            }
                            Err(e) => self.tui_err(format!("  Failed to enable skill: {e}")),
                        }
                    }
                }

                "unload" => {
                    let id = sub_arg.trim();
                    if id.is_empty() {
                        self.tui_err("  Usage: /skills unload <skill_name>");
                    } else {
                        let agent_id = self.agent_id();
                        let existing = self.client.get_memory(&agent_id).await.unwrap_or_default();
                        
                        let mut skill_removed = false;
                        let mut project_updated = false;

                        if let Some(project_block) = existing.iter().find(|b| b.label == "project") {
                            let mut lines = Vec::new();
                            let mut in_section = false;
                            
                            for line in project_block.value.lines() {
                                let trimmed = line.trim();
                                if trimmed.starts_with("## Required Skills") || trimmed.starts_with("## Required skills") {
                                    in_section = true;
                                    lines.push(line.to_string());
                                    continue;
                                }
                                if in_section && trimmed.starts_with("## ") {
                                    in_section = false;
                                }
                                
                                if in_section && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
                                    let clean = trimmed[2..].trim();
                                    let parsed_id = clean.split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
                                    if parsed_id.to_lowercase() == id.to_lowercase() {
                                        skill_removed = true;
                                        continue;
                                    }
                                }
                                lines.push(line.to_string());
                            }
                            
                            let new_value = lines.join("\n");
                            if skill_removed {
                                match self.client.upsert_memory(&agent_id, "project", &new_value, None).await {
                                    Ok(()) => {
                                        self.tui_ok(format!("  ✓ Removed skill '{id}' from [project] memory block."));
                                        project_updated = true;
                                    }
                                    Err(e) => {
                                        self.tui_err(format!("  Failed to update [project] memory block on server: {e}"));
                                    }
                                }
                            } else {
                                self.tui_dim(format!("  Info: Skill '{id}' was not found in [project] memory's required skills list."));
                            }
                        } else {
                            self.tui_dim("  Info: No [project] memory block found for this agent.");
                        }

                        // Also disable/exclude on server so it doesn't get loaded dynamically in this turn
                        match self.client.disable_skill_on_server(&agent_id, id).await {
                            Ok(()) => {
                                self.tui_ok(format!(
                                    "  ✓ Skill '{id}' disabled — excluded from active context."
                                ));
                            }
                            Err(e) => {
                                if !project_updated && !skill_removed {
                                    self.tui_err(format!("  Failed to disable skill: {e}"));
                                } else {
                                    tracing::debug!("Failed to disable skill on server: {e}");
                                }
                            }
                        }
                    }
                }

                other => {
                    self.tui_err(format!("  Unknown /skills subcommand: '{other}'"));
                    self.tui_blank();
                    self.tui_dim("  /skills                    — open interactive skills manager");
                    self.tui_dim("  /skills new <name>         — scaffold a new skill");
                    self.tui_dim("  /skills delete <id>        — remove a skill directory");
                    self.tui_dim(
                        "  /skills disable <id>       — exclude skill from context (keeps files)",
                    );
                    self.tui_dim("  /skills enable <id>        — re-include a disabled skill");
                    self.tui_dim("  /skills unload <id>        — remove skill from [project] memory and exclude it");
                    self.tui_dim("  /skills reload             — rescan all skill directories");
                    self.tui_blank();
                }
            }
        }

        Ok(false)
    }
}

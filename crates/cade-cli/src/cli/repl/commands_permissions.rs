//! /permissions command handler.

use super::{Repl, mode_display};
use crate::Result;
use crate::ui::RenderLine;

impl Repl {
    pub(crate) async fn cmd_permissions(&mut self) -> Result<bool> {
        let mode = self.permissions.mode();
        let allow = self.permissions.allow_rules();
        let deny = self.permissions.deny_rules();
        let (icon, label, _) = mode_display(mode);
        let mode_hint = match mode {
            cade_core::permissions::PermissionMode::Default => "ask before each tool call",
            cade_core::permissions::PermissionMode::AcceptEdits => {
                "file edits auto-approved; Bash still prompts"
            }
            cade_core::permissions::PermissionMode::Plan => "read-only; write operations blocked",
            cade_core::permissions::PermissionMode::BypassPermissions => {
                "all tools auto-approved (deny rules still apply)"
            }
        };
        self.tui_blank();
        self.tui_hdr(format!("  Mode: {icon} {label}  —  {mode_hint}"));
        self.tui_blank();
        if allow.is_empty() && deny.is_empty() {
            self.tui_dim("  No allow/deny rules active.");
        } else {
            if !allow.is_empty() {
                self.tui_ok(format!("  Allow rules ({}):", allow.len()));
                for r in &allow {
                    self.tui_dim(format!("    {:<12} {}", r.tool(), r.arg_display()));
                }
                let _ = self.app.lock().push(RenderLine::Blank);
            }
            if !deny.is_empty() {
                self.tui_err(format!("  Deny rules ({}):", deny.len()));
                for r in &deny {
                    self.tui_dim(format!("    {:<12} {}", r.tool(), r.arg_display()));
                }
                self.tui_blank();
            }
        }
        self.tui_dim("  /approve-always <pattern>    /deny-always <pattern>");
        self.tui_dim("  Pattern:  Bash(cargo test)  ·  Read(src/**)  ·  Bash(rm -rf:*)");
        Ok(false)
    }

    pub(crate) async fn cmd_approve_always(&mut self, pattern: String) -> Result<bool> {
        if pattern.is_empty() {
            self.tui_dim("  /approve-always <pattern>");
            self.tui_dim("  Examples:  Bash(cargo test)  Read(src/**)  Bash(git commit:*)  Bash");
        } else if let Some(rule) = cade_core::permissions::PermissionRule::parse(&pattern) {
            self.permissions.add_allow_rule(rule.clone());
            self.tui_ok(format!(
                "  ✓ Allow  {:<12} {}",
                rule.tool(),
                rule.arg_display()
            ));
            use crate::ui::question::{Question, QuestionOption};
            let opts = vec![
                QuestionOption {
                    label: "Yes — save to settings.json".to_string(),
                    description: String::new(),
                },
                QuestionOption {
                    label: "No — session only".to_string(),
                    description: String::new(),
                },
            ];
            let q = Question {
                header: "Save rule?".to_string(),
                text: "Persist this rule to settings.json?".to_string(),
                options: opts.clone(),
                multi_select: false,
                allow_other: false,
                progress: None,
            };
            let save = {
                let mut app = self.app.lock();
                let r = app.ask_question(&q)?;
                app.scroll = 0;
                let _ = app.draw();
                matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
            };
            if save {
                let mut settings = self.settings.lock();
                let res: std::result::Result<(), cade_core::Error> =
                    settings.save_allow_rule(&pattern);
                match res {
                    Ok(_) => self.tui_ok("  ✓ Saved"),
                    Err(e) => self.tui_err(e.to_string()),
                }
            }
        } else {
            self.tui_err(format!(
                "invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"
            ));
        }
        Ok(false)
    }

    pub(crate) async fn cmd_deny_always(&mut self, pattern: String) -> Result<bool> {
        if pattern.is_empty() {
            self.tui_dim("  /deny-always <pattern>");
            self.tui_dim("  Examples:  Bash(rm -rf:*)  Bash(git push --force)  Bash");
        } else if let Some(rule) = cade_core::permissions::PermissionRule::parse(&pattern) {
            self.permissions.add_deny_rule(rule.clone());
            self.tui_err(format!(
                "  ✗ Deny   {:<12} {}",
                rule.tool(),
                rule.arg_display()
            ));
            use crate::ui::question::{Question, QuestionOption};
            let opts = vec![
                QuestionOption {
                    label: "Yes — save to settings.json".to_string(),
                    description: String::new(),
                },
                QuestionOption {
                    label: "No — session only".to_string(),
                    description: String::new(),
                },
            ];
            let q = Question {
                header: "Save rule?".to_string(),
                text: "Persist this rule to settings.json?".to_string(),
                options: opts.clone(),
                multi_select: false,
                allow_other: false,
                progress: None,
            };
            let save = {
                let mut app = self.app.lock();
                let r = app.ask_question(&q)?;
                app.scroll = 0;
                let _ = app.draw();
                matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
            };
            if save {
                let mut settings = self.settings.lock();
                let res: std::result::Result<(), cade_core::Error> =
                    settings.save_deny_rule(&pattern);
                match res {
                    Ok(_) => self.tui_ok("  ✓ Saved"),
                    Err(e) => self.tui_err(e.to_string()),
                }
            }
        } else {
            self.tui_err(format!(
                "invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"
            ));
        }
        Ok(false)
    }
}

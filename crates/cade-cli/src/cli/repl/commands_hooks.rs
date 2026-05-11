//! /hooks command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_hooks(&mut self) -> Result<bool> {
        let merged = self.settings.lock().merged_hooks();
        self.tui_blank();
        if merged.is_empty() {
            self.tui_dim("  No hooks configured.");
            self.tui_dim("  Configure in ~/.cade/settings.json or .cade/settings.json");
            self.tui_blank();
            self.tui_dim("  Example: { \"hooks\": { \"PreToolUse\": [{ \"matcher\": \"Bash\", \"hooks\": [{ \"type\": \"command\", \"command\": \"./validate.sh\" }] }] } }");
            self.tui_dim("  Exit codes:  0=allow  1=log+continue  2=block (stderr→agent)");
        } else {
            self.tui_hdr("  Hooks");
            self.tui_blank();
            let show_section = |name: &str, entries: &[cade_core::settings::HookEntry]| {
                if !entries.is_empty() {
                    self.tui_hdr(format!("  {name}  ({}):", entries.len()));
                    for entry in entries {
                        let m = entry.matcher.as_deref().unwrap_or("*");
                        self.tui_dim(format!("    matcher: {m}"));
                        for hook in &entry.hooks {
                            self.tui_dim(format!("      {hook}"));
                        }
                    }
                    self.tui_blank();
                }
            };
            show_section("PreToolUse", &merged.pre_tool_use);
            show_section("PostToolUse", &merged.post_tool_use);
            show_section("PostToolUseFailure", &merged.post_tool_use_failure);
            show_section("PermissionRequest", &merged.permission_request);
            show_section("UserPromptSubmit", &merged.user_prompt_submit);
            show_section("Stop", &merged.stop);
            show_section("SubagentStop", &merged.subagent_stop);
            show_section("SessionStart", &merged.session_start);
            show_section("SessionEnd", &merged.session_end);
            show_section("Notification", &merged.notification);
            self.tui_dim("  Config: ~/.cade/settings.json  ·  .cade/settings.json  ·  .cade/settings.local.json");
        }
        Ok(false)
    }
}

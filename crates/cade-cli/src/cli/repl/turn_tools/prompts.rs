use super::super::Repl;
use super::super::turn_loop::now_epoch_ms;
use crate::Result;
use crate::support::text::truncate;
use crate::ui::RenderLine;
use std::io;

impl Repl {
    pub(crate) async fn prompt_approval(
        &self,
        _stdout: &mut io::Stdout,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool> {
        use crate::ui::question::{Question, QuestionOption};

        // Show diff preview for file-mutation tools before the approval prompt.
        if let Some(diff_lines) = Self::build_diff_preview(tool_name, args) {
            let mut app = self.app.lock();
            for line in diff_lines {
                let _ = app.push(line);
            }
            let _ = app.draw();
        }

        // One-line preview of what is being requested
        let preview: String = if let Some(cmd) = args["command"].as_str() {
            truncate(cmd, 100).to_string()
        } else if let Some(fp) = args["file_path"].as_str().or(args["path"].as_str()) {
            fp.to_string()
        } else if let Some(pat) = args["pattern"].as_str() {
            format!("\"{}\"", truncate(pat, 60))
        } else {
            String::new()
        };

        // Header chip — tool name, max 12 chars
        let header_raw = tool_name.replace('_', " ");
        let header: String = header_raw.chars().take(12).collect();

        let mut warning_text = String::new();
        if tool_name == "bash"
            && let Some(cmd) = args["command"].as_str()
            && cade_core::permissions::bash_command_is_suspicious(cmd)
        {
            warning_text = "\n⚠️  WARNING: Suspicious command detected (nested shell, network, or obfuscation)".to_string();
        }

        let question_text = if preview.is_empty() {
            format!("Run {tool_name}?{warning_text}")
        } else {
            format!("{preview}{warning_text}")
        };

        let opts = vec![
            QuestionOption {
                label: "Yes".to_string(),
                description: "Run this tool once".to_string(),
            },
            QuestionOption {
                label: "Yes, don't ask again".to_string(),
                description: "Allow this tool for the rest of the session".to_string(),
            },
            QuestionOption {
                label: "No".to_string(),
                description: "Deny this tool call".to_string(),
            },
        ];

        let q = Question {
            header: header.clone(),
            text: question_text.clone(),
            options: opts.clone(),
            multi_select: false,
            allow_other: false,
            progress: None,
        };

        let rx = {
            let mut app = self.app.lock();
            app.ask_question_async(q)?
        };

        let qa = rx
            .await
            .map_err(|e| crate::Error::custom(format!("approval channel dropped: {e}")))?;
        // Record close time so the tick task's I-01 Enter handler can apply
        // a 300 ms grace period (mirrors the 200 ms Esc grace period).
        self.last_modal_close_ms
            .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);

        match qa {
            None => {
                // Esc / Ctrl+C = deny. Clear any cancel flag set while the
                // blocking question was active — an Esc inside the modal must
                // not abort the subsequent stream_turn.
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                Ok(false)
            }
            Some(answer) => {
                let label = answer.as_str();
                // Clear any stale SIGINT cancel flag set while the blocking
                // event loop ran (terminal may have converted Ctrl+Enter or
                // a buffered Esc into an OS-level interrupt during the modal).
                // Without this reset the next stream_turn would see
                // cancel_turn == true and immediately abort with "Turn interrupted".
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                if label.starts_with("Yes, don't") {
                    // Store allow rule BEFORE returning so that any immediately
                    // following tool call of the same type is auto-approved (B3).
                    self.permissions.add_session_allow(tool_name);
                    Ok(true)
                } else if label.starts_with("Yes") {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Interactive `ask_user_question` tool intercept.
    ///
    /// Parses the LLM's structured questions, shows the `QuestionWidget` for
    /// each one sequentially, then returns a formatted result string to the agent.
    pub(crate) async fn handle_ask_user_question(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        use crate::ui::question::{Question, QuestionOption};
        use cade_agent::tools::AskUserQuestionTool;
        use std::collections::HashMap;

        // Parse and validate
        let ask_questions = match AskUserQuestionTool::parse_questions(args) {
            Ok(q) => q,
            Err(e) => {
                let msg = format!("Invalid ask_user_question args: {e}");
                let _ = self.app.lock().push(RenderLine::ToolResult {
                    is_error: true,
                    content: msg.clone(),
                });
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "ask_user_question".to_string(),
                    output: msg,
                    is_error: true,
                    ui_resource_uri: None,
                });
            }
        };

        let total = ask_questions.len();
        let _ = self.app.lock().commit_streaming();

        let mut answers: HashMap<String, String> = HashMap::new();
        let mut answers_display: Vec<(String, String)> = Vec::new();

        for (i, aq) in ask_questions.iter().enumerate() {
            let opts: Vec<QuestionOption> = aq
                .options
                .iter()
                .map(|o| QuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect();

            let q = Question {
                header: aq.header.clone(),
                text: aq.question.clone(),
                options: opts.clone(),
                multi_select: aq.multi_select,
                allow_other: true,
                progress: if total > 1 {
                    Some((i + 1, total))
                } else {
                    None
                },
            };

            let rx = {
                let mut app = self.app.lock();
                app.ask_question_async(q)?
            };

            let qa = rx.await.map_err(|e| {
                crate::Error::custom(format!("ask_user_question channel dropped: {e}"))
            })?;

            self.last_modal_close_ms
                .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);

            match qa {
                None => {
                    // User cancelled — clear any stale cancel flag so subsequent
                    // stream_turn calls are not aborted immediately.
                    self.cancel_turn
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    let msg = "User cancelled the question prompt.".to_string();
                    let _ = self.app.lock().push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                    return Ok(cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "ask_user_question".to_string(),
                        output: msg,
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
                Some(answer) => {
                    answers_display.push((aq.header.clone(), answer.as_str()));
                    answers.insert(aq.question.clone(), answer.as_str());
                }
            }
        }

        // Show answers inline under the tool call header (⎿ answer / ⎿ h: a\n  h: b)
        let result_content = if total == 1 {
            answers_display[0].1.clone()
        } else {
            answers_display
                .iter()
                .map(|(h, a)| format!("{h}: {a}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Clear any stale cancel flag accumulated during the question loop so
        // the following stream_turn is not aborted prematurely.
        self.cancel_turn
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Removed internal ToolResult push since dispatch_tool_calls pushes it unconditionally.
        {
            let mut app = self.app.lock();
            // Force a redraw to ensure the viewport updates immediately after the
            // question modal is dismissed, fixing a race condition where the
            // result of the next tool call would not be displayed.
            let _ = app.draw();
        }

        Ok(cade_agent::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "ask_user_question".to_string(),
            output: result_content,
            is_error: false,
            ui_resource_uri: None,
        })
    }

    /// Build a compact argument preview for a tool call header.
    pub(crate) fn tool_preview(_tool_name: &str, args: &serde_json::Value) -> String {
        fn short(s: &str, n: usize) -> String {
            let s = s.trim();
            if s.chars().count() <= n {
                s.to_string()
            } else {
                format!("{}…", s.chars().take(n).collect::<String>())
            }
        }
        let a = args;
        if let Some(cmd) = a["command"].as_str() {
            short(cmd, 80)
        } else if let Some(fp) = a["file_path"].as_str().or(a["path"].as_str()) {
            let extra = if let Some(old) = a["old_string"].as_str() {
                format!("  \"{}\"", short(old, 40))
            } else if let Some(content) = a["content"].as_str() {
                format!("  ({} chars)", content.len())
            } else {
                String::new()
            };
            format!("{fp}{extra}")
        } else if let Some(pat) = a["pattern"].as_str() {
            let in_path = a["path"].as_str().unwrap_or("");
            if in_path.is_empty() {
                format!("\"{}\"", short(pat, 60))
            } else {
                format!("\"{}\" in {in_path}", short(pat, 40))
            }
        } else if let Some(label) = a["label"].as_str() {
            let op = a["operation"].as_str().unwrap_or("set");
            format!("[{label}] ({op})")
        } else if let Some(patch) = a["patch"].as_str() {
            short(patch, 60)
        } else {
            a.as_object()
                .and_then(|m| m.values().find_map(|v| v.as_str()).map(|s| short(s, 60)))
                .unwrap_or_default()
        }
    }

    /// Prompt the user to approve/deny a tool call.
    /// Returns true = approved, false = denied.
    ///
    /// Shows a ratatui inline menu with three options:
    ///   1. Yes — run once
    ///   2. Yes, don't ask again — session-allow + run
    ///   3. No — deny
    ///      Generate a diff preview for file-mutation tools shown before the approval prompt.
    pub(crate) fn build_diff_preview(
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<Vec<RenderLine>> {
        match tool_name {
            "edit_file" => {
                let path = args["path"].as_str()?;
                let old_string = args["old_string"].as_str()?;
                let new_string = args["new_string"].as_str()?;
                let existing = std::fs::read_to_string(path).ok()?;
                let offset = existing
                    .find(old_string)
                    .map(|byte| existing[..byte].lines().count())
                    .unwrap_or(0);
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg(format!("--- {path}"))];
                for (i, ln) in old_string.lines().enumerate() {
                    out.push(RenderLine::ErrorMsg(format!(
                        "- {ln}  (L{})",
                        offset + i + 1
                    )));
                }
                for ln in new_string.lines() {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                Some(out)
            }
            "write_file" | "create_file" => {
                let path = args["path"].as_str()?;
                let content = args["content"].as_str()?;
                let is_new = !std::path::Path::new(path).exists();
                let lines: Vec<&str> = content.lines().collect();
                let show = lines.len().min(12);
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg(format!(
                    "{} {path}",
                    if is_new { "new file:" } else { "overwrite:" }
                ))];
                for ln in &lines[..show] {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                if lines.len() > show {
                    out.push(RenderLine::DimMsg(format!(
                        "  … ({} more lines)",
                        lines.len() - show
                    )));
                }
                Some(out)
            }
            "apply_patch" => {
                let patch = args["patch"].as_str()?;
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg("(patch)".to_string())];
                for ln in patch.lines().take(20) {
                    if ln.starts_with('-') && !ln.starts_with("---") {
                        out.push(RenderLine::ErrorMsg(ln.to_string()));
                    } else if ln.starts_with('+') && !ln.starts_with("+++") {
                        out.push(RenderLine::SuccessMsg(ln.to_string()));
                    } else {
                        out.push(RenderLine::DimMsg(ln.to_string()));
                    }
                }
                if patch.lines().count() > 20 {
                    out.push(RenderLine::DimMsg(format!(
                        "… ({} more lines)",
                        patch.lines().count() - 20
                    )));
                }
                Some(out)
            }
            _ => None,
        }
    }
}

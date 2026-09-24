use super::Repl;
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};
use crate::Result;
use crate::support::text::{FinishReasonCategory, finish_reason_hint};
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

/// Build a compact one-line argument preview for a tool call header row.
fn tool_args_preview(args: &serde_json::Value) -> String {
    fn short(s: &str, n: usize) -> String {
        let s = s.trim();
        if s.chars().count() <= n {
            s.to_string()
        } else {
            format!("{}…", s.chars().take(n).collect::<String>())
        }
    }
    if let Some(cmd) = args["command"].as_str() {
        short(cmd, 80)
    } else if let Some(fp) = args["file_path"].as_str().or(args["path"].as_str()) {
        let extra = if let Some(old) = args["old_string"].as_str() {
            format!("  \"{}\"", short(old, 40))
        } else if let Some(content) = args["content"].as_str() {
            format!("  ({} chars)", content.len())
        } else {
            String::new()
        };
        format!("{fp}{extra}")
    } else if let Some(pat) = args["pattern"].as_str() {
        let in_path = args["path"].as_str().unwrap_or("");
        if in_path.is_empty() {
            format!("\"{}\"", short(pat, 60))
        } else {
            format!("\"{}\" in {in_path}", short(pat, 40))
        }
    } else if let Some(label) = args["label"].as_str() {
        let op = args["operation"].as_str().unwrap_or("set");
        format!("[{label}] ({op})")
    } else if let Some(patch) = args["patch"].as_str() {
        short(patch, 60)
    } else {
        args.as_object()
            .and_then(|m| m.values().find_map(|v| v.as_str()).map(|s| short(s, 60)))
            .unwrap_or_default()
    }
}

impl Repl {
    /// Stream one turn (user message or tool return) and render live.
    /// Returns the complete collected message list.
    ///
    /// `bar_text`: optional shared string updated by tool_call_message events
    /// to keep the ThinkingBar status current.
    pub(crate) async fn stream_turn(
        &mut self,
        _stdout: &mut io::Stdout,
        input: &str,
        is_tool_return: bool,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &str,
        ephemeral: bool,
        _spinner: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        bar_text: Option<std::sync::Arc<parking_lot::Mutex<String>>>,
    ) -> Result<Vec<CadeMessage>> {
        // -- R-04: Async event buffering
        // Decouples network I/O from TUI rendering.  The SSE callback (`on_event`)
        // performs only lightweight session/stats bookkeeping and forwards each
        // message to an unbounded channel.  A dedicated UI consumer task reads
        // from the channel and applies all TuiApp mutations + draws.  This means
        // the SSE event loop is never blocked by draw() or lock contention.

        // -- Per-turn channel
        let (ui_tx, ui_rx) = tokio::sync::mpsc::unbounded_channel::<CadeMessage>();

        // -- Session / stats state (used by on_event — NO TuiApp access)
        let conv_arc = self.conversation_id.clone();
        let session_arc = self.session.clone();
        let sess_in_tok = self.session_input_tokens.clone();
        let sess_out_tok = self.session_output_tokens.clone();
        let sess_stats = self.session_stats.clone();
        let run_id_cell: std::sync::Arc<parking_lot::Mutex<Option<String>>> = Default::default();
        let seq_id_cell: std::sync::Arc<parking_lot::Mutex<Option<i64>>> = Default::default();
        let run_id_cell2 = run_id_cell.clone();
        let seq_id_cell2 = seq_id_cell.clone();
        let finish_reason_arc: std::sync::Arc<parking_lot::Mutex<Option<String>>> =
            Default::default();
        let finish_reason_cb = finish_reason_arc.clone();

        // -- on_event: SSE callback — stats only, then forward to UI channel
        let on_event = move |msg: &CadeMessage| {
            match msg.msg_type() {
                "stream_start" => {
                    if let Some(cid) = msg.data["conversation_id"].as_str()
                        && !cid.is_empty()
                        && conv_arc.lock().as_deref() != Some(cid)
                    {
                        let cid: String = cid.to_string();
                        *conv_arc.lock() = Some(cid.clone());
                        {
                            let mut s = session_arc.lock();
                            let _ = s.set_conversation(Some(cid));
                        }
                    }
                    if let Some(rid) = msg.run_id() {
                        *run_id_cell2.lock() = Some(rid.to_string());
                    }
                }
                "usage_statistics" => {
                    use std::sync::atomic::Ordering;
                    if let Some(n) = msg.data["input_tokens"].as_u64() {
                        sess_in_tok.fetch_add(n, Ordering::SeqCst);
                    }
                    if let Some(n) = msg.data["output_tokens"].as_u64() {
                        sess_out_tok.fetch_add(n, Ordering::SeqCst);
                    }
                    {
                        let mut stats = sess_stats.lock();
                        let model = msg.data["model"].as_str().unwrap_or("").to_string();
                        let input = msg.data["input_tokens"].as_u64().unwrap_or(0);
                        let cache_read = msg.data["cache_read_tokens"].as_u64().unwrap_or(0);
                        let cache_write = msg.data["cache_write_tokens"].as_u64().unwrap_or(0);
                        let output = msg.data["output_tokens"].as_u64().unwrap_or(0);
                        stats.record_usage(&model, input, cache_read, cache_write, output);
                    }
                }
                "finish_reason" => {
                    if let Some(reason) = msg.data["reason"].as_str() {
                        *finish_reason_cb.lock() = Some(reason.to_string());
                    }
                }
                _ => {}
            }
            if let Some(s) = msg.seq_id() {
                *seq_id_cell2.lock() = Some(s);
            }
            // Forward to UI consumer (non-blocking, never stalls the SSE loop).
            let _ = ui_tx.send(msg.clone());
        };

        // -- UI consumer task — all TuiApp mutations happen here
        let app_arc = self.app.clone();
        let bar_text_arc = bar_text;
        let reasoning_buf = self.last_reasoning.clone();
        let assistant_buf = self.last_assistant_text.clone();
        // Session-level stats for footer metrics (tokens, cost, cache usage)
        let sess_in_tok_ui = self.session_input_tokens.clone();
        let sess_out_tok_ui = self.session_output_tokens.clone();
        let sess_stats_ui = self.session_stats.clone();
        // Full model ID (provider/name) for accurate context window lookup.
        // The usage event's `model` field carries only the bare name (after
        // the LlmRouter strips the provider prefix), which causes
        // context_window_for_model to fall through to a wrong default.
        let full_model_id = self.model();
        // Clear buffers at the start of each turn.
        reasoning_buf.lock().clear();
        assistant_buf.lock().clear();
        let client_for_ui = self.client.clone();
        let ui_task = tokio::spawn(async move {
            let mut ui_rx = ui_rx;
            let mut in_reasoning = false;
            while let Some(msg) = ui_rx.recv().await {
                match msg.msg_type() {
                    "reasoning_message" => {
                        if let Some(text) = msg.reasoning_text() {
                            in_reasoning = true;
                            reasoning_buf.lock().push_str(text);
                            app_arc.lock().push_reasoning_chunk(text);
                        }
                    }
                    "assistant_message" => {
                        if let Some(text) = msg.assistant_text() {
                            assistant_buf.lock().push_str(text);
                            if !text.is_empty() {
                                in_reasoning = false;
                                let line_count = {
                                    let mut app = app_arc.lock();
                                    app.commit_reasoning_inner();
                                    let _ = app.push_streaming_chunk(text);
                                    app.lines.len()
                                };
                                if let Some(bar) = &bar_text_arc {
                                    let cur = bar.lock().clone();
                                    if !cur.starts_with("●") {
                                        *bar.lock() = format!("generating… ({line_count} lines)");
                                    }
                                }
                            }
                        } else if in_reasoning {
                            let _ = app_arc.lock().commit_reasoning();
                            in_reasoning = false;
                        }
                    }
                    "tool_call_message" => {
                        in_reasoning = false;
                        let (_tool_id, tool_name, args) = match msg.as_tool_call() {
                            Some(t) => t,
                            None => continue,
                        };
                        let preview = tool_args_preview(&args);
                        {
                            let mut app = app_arc.lock();
                            let _ = app.push(RenderLine::ToolCall {
                                name: tool_name.clone(),
                                preview,
                            });
                        }
                        if let Some(bar) = &bar_text_arc {
                            let display = if let Some(pos) = tool_name.rfind("__") {
                                &tool_name[pos + 2..]
                            } else {
                                &tool_name
                            };
                            *bar.lock() = format!("● {}…", display);
                        }
                    }
                    "tool_result_message" => {
                        let content = msg.data["tool_result"]["output"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let is_error = msg.data["tool_result"]["is_error"]
                            .as_bool()
                            .unwrap_or(false);
                        let _ = app_arc
                            .lock()
                            .push(RenderLine::ToolResult { is_error, content });
                    }
                    "usage_statistics" => {
                        use std::sync::atomic::Ordering;

                        // Stats already updated in on_event; here we derive UI metrics:
                        // - session tokens (↑ input, ↓ output)
                        // - cache tokens (R read, W write)
                        // - total cost (USD)
                        // - context usage % and window size
                        // - current permission mode (auto/edits/plan/yolo)
                        //
                        // Use the full model ID (provider/name) for the context
                        // window lookup.  The usage event's `model` field carries
                        // only the bare name (router strips the prefix), which
                        // causes context_window_for_model to fall through to a
                        // wrong 32k default for dynamic/uncatalogued models.
                        let _model = msg.data["model"].as_str().unwrap_or("");
                        let input = msg.data["input_tokens"].as_u64().unwrap_or(0);
                        let cache_read = msg.data["cache_read_tokens"].as_u64().unwrap_or(0);
                        let window = cade_ai::catalogue::context_window_for_model(&full_model_id);

                        // Per-turn context usage for this model
                        let (pct_f_opt, pct_int_opt) = if window > 0 {
                            let used = input + cache_read;
                            let pct_f = (used as f64 / window as f64) * 100.0;
                            let pct_int = pct_f.round().min(99.0) as u8;
                            (Some(pct_f), Some(pct_int))
                        } else {
                            (None, None)
                        };

                        // Session-level aggregates
                        let in_tok = sess_in_tok_ui.load(Ordering::SeqCst);
                        let out_tok = sess_out_tok_ui.load(Ordering::SeqCst);
                        let (cache_r, cache_w, total_cost) = {
                            let stats = sess_stats_ui.lock();
                            let cache_r: u64 =
                                stats.per_model.values().map(|m| m.cache_read_tokens).sum();
                            let cache_w: u64 =
                                stats.per_model.values().map(|m| m.cache_write_tokens).sum();
                            let (total_cost, _) = stats.compute_cost();
                            (cache_r, cache_w, total_cost)
                        };

                        // Update TUI context_pct and footer_extra in one lock
                        let mut app = app_arc.lock();
                        if let Some(pct_int) = pct_int_opt {
                            app.set_context_pct(pct_int);
                        }
                        let ctx_pct_f = pct_f_opt
                            .unwrap_or_else(|| app.context_pct.map(|p| p as f64).unwrap_or(0.0));
                        let window_str = fmt_window_tokens_short(window);
                        let mode_label = short_mode_label(app.mode);

                        let metrics = format!(
                            "↑{} ↓{} R{} W{} ${:.3} {:.1}%/{} ({})",
                            fmt_tok_short(in_tok),
                            fmt_tok_short(out_tok),
                            fmt_tok_short(cache_r),
                            fmt_tok_short(cache_w),
                            total_cost,
                            ctx_pct_f,
                            window_str,
                            mode_label,
                        );
                        app.footer_extra = Some(metrics);
                        app.session_cost_usd = total_cost;
                    }
                    "system_notice" => {
                        // Phase 3: server-side overflow recovery (and
                        // similar) surface a user-visible message via
                        // SSE.  Show as a toast and mirror to the timeline
                        // so it appears in session export/copy.
                        let level = msg.data["level"].as_str().unwrap_or("info");
                        let text = msg.data["message"].as_str().unwrap_or("").to_string();
                        if !text.is_empty() {
                            let toast_level = match level {
                                "error" => crate::ui::ToastLevel::Error,
                                "warning" => crate::ui::ToastLevel::Warning,
                                "success" => crate::ui::ToastLevel::Success,
                                _ => crate::ui::ToastLevel::Info,
                            };
                            let mut app = app_arc.lock();
                            app.show_toast(text.clone(), toast_level);
                            let _ = app.push(cade_tui::RenderLine::SystemMsg(text));
                        }
                    }
                    "plan_update" => {
                        let mut app = app_arc.lock();
                        if let Some(plan) = msg.data.get("plan") {
                            let steps: Vec<String> = plan
                                .get("steps")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|step| {
                                            step.get("description")
                                                .and_then(|d| d.as_str())
                                                .map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            if !steps.is_empty() {
                                app.set_plan(steps);
                            }

                            // Apply step status updates (done/pending)
                            if let Some(steps_arr) = plan.get("steps").and_then(|v| v.as_array()) {
                                for step in steps_arr {
                                    let id = step.get("id").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as usize;
                                    if let Some(done) =
                                        step.get("is_done").and_then(|v| v.as_bool())
                                    {
                                        app.update_plan_step(id, done);
                                    }
                                }
                            }
                        }
                    }
                    "tool_progress_message" => {
                        if let Some(progress) = msg.data.get("tool_progress") {
                            let tool_name = progress["name"].as_str().unwrap_or("tool");
                            let status = progress["status"].as_str().unwrap_or("started");
                            let message = progress["message"].as_str().unwrap_or("");
                            if let Some(bar) = &bar_text_arc {
                                if status == "started" {
                                    *bar.lock() = format!("● {}…", tool_name);
                                } else if status == "completed" {
                                    *bar.lock() = "● processing…".to_string();
                                }
                            }
                            if !message.is_empty() {
                                let mut app = app_arc.lock();
                                app.show_toast(message.to_string(), crate::ui::ToastLevel::Info);
                            }
                        }
                    }
                    "error" => {
                        if let Some(err) = msg.data.get("error").and_then(|v| v.as_str()) {
                            if let Some(bar) = &bar_text_arc {
                                *bar.lock() = format!("✗ Error: {err}");
                            }
                            let mut app = app_arc.lock();
                            let _ = app.commit_reasoning();
                            let _ = app.commit_streaming();
                            app.show_toast(err.to_string(), crate::ui::ToastLevel::Error);
                            let _ = app.push(cade_tui::RenderLine::ErrorMsg(err.to_string()));
                            app.set_last_status(Some(format!("✗ Error: {err}")));
                            app.draw_dirty = true;
                            let _ = app.draw();
                        }
                    }
                    "approval_requested" | "approval_required" => {
                        let id = msg.data["id"].as_str().unwrap_or("").to_string();
                        let tool = msg.data["tool_name"].as_str().unwrap_or("tool").to_string();
                        let reason = msg.data["reason"]
                            .as_str()
                            .unwrap_or("requires permission")
                            .to_string();
                        let args_val = msg
                            .data
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        let subagent = msg.data.get("subagent_id").and_then(|v| v.as_str());

                        if let Some(subagent_id) = subagent {
                            let text = format!(
                                "⚠️ Background Subagent [{}] requests permission to run {}. Type /approvals to review.",
                                subagent_id, tool
                            );
                            let mut app = app_arc.lock();
                            app.show_toast(text.clone(), crate::ui::ToastLevel::Warning);
                            let _ = app.push(RenderLine::SystemMsg(text.clone()));
                        } else if !id.is_empty() {
                            let client_c = client_for_ui.clone();
                            let approval_id_c = id.clone();
                            let target_preview = args_val
                                .get("command")
                                .or_else(|| args_val.get("path"))
                                .or_else(|| args_val.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            let question = cade_tui::question::Question {
                                header: format!("Approve {tool}"),
                                text: if target_preview.is_empty() {
                                    format!("Allow tool '{tool}' to run?\nReason: {reason}")
                                } else {
                                    format!(
                                        "Allow tool '{tool}' to run?\nTarget: {target_preview}\nReason: {reason}"
                                    )
                                },
                                options: vec![
                                    cade_tui::question::QuestionOption {
                                        label: "Allow once".to_string(),
                                        description: "Approve this single tool execution".to_string(),
                                    },
                                    cade_tui::question::QuestionOption {
                                        label: "Allow for session".to_string(),
                                        description:
                                            "Auto-approve this tool for the remainder of this session"
                                                .to_string(),
                                    },
                                    cade_tui::question::QuestionOption {
                                        label: "Deny".to_string(),
                                        description: "Reject execution of this tool".to_string(),
                                    },
                                ],
                                multi_select: false,
                                allow_other: false,
                                progress: None,
                            };

                            let rx_opt = {
                                let mut app = app_arc.lock();
                                app.show_toast(
                                    format!("🔒 Approval required for {tool}"),
                                    crate::ui::ToastLevel::Warning,
                                );
                                app.ask_question_async(question).ok()
                            };

                            if let Some(rx) = rx_opt {
                                tokio::spawn(async move {
                                    let action = match rx.await {
                                        Ok(Some(cade_tui::question::QuestionAnswer::Single(
                                            ref label,
                                        ))) if label == "Allow once"
                                            || label == "Allow for session" =>
                                        {
                                            "approve"
                                        }
                                        _ => "deny",
                                    };
                                    let body = serde_json::json!({ "action": action });
                                    let _ = client_c
                                        .raw_post(
                                            &format!("/approvals/{approval_id_c}/action"),
                                            &body,
                                        )
                                        .await;
                                });
                            }
                        }
                    }
                    "question_required" | "question_requested" => {
                        let id = msg.data["id"].as_str().unwrap_or("").to_string();
                        let questions_val = msg
                            .data
                            .get("questions")
                            .cloned()
                            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));

                        if !id.is_empty()
                            && let Some(first_q) =
                                questions_val.as_array().and_then(|arr| arr.first())
                        {
                            let header = first_q
                                .get("header")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Question")
                                .to_string();
                            let text = first_q
                                .get("question")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let multi_select = first_q
                                .get("multiSelect")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let options = first_q
                                .get("options")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|opt| {
                                            let label = opt
                                                .get("label")
                                                .and_then(|v| v.as_str())?
                                                .to_string();
                                            let desc = opt
                                                .get("description")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            Some(cade_tui::question::QuestionOption {
                                                label,
                                                description: desc,
                                            })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            let question = cade_tui::question::Question {
                                header,
                                text,
                                options,
                                multi_select,
                                allow_other: true,
                                progress: None,
                            };

                            let client_c = client_for_ui.clone();
                            let question_id_c = id.clone();

                            let rx_opt = {
                                let mut app = app_arc.lock();
                                app.show_toast(
                                    "❓ Clarifying question from agent".to_string(),
                                    crate::ui::ToastLevel::Info,
                                );
                                app.ask_question_async(question).ok()
                            };

                            if let Some(rx) = rx_opt {
                                tokio::spawn(async move {
                                    let (action, feedback) = match rx.await {
                                        Ok(Some(cade_tui::question::QuestionAnswer::Single(
                                            ref label,
                                        ))) => ("approve", Some(label.clone())),
                                        Ok(Some(cade_tui::question::QuestionAnswer::Multi(
                                            ref labels,
                                        ))) => ("approve", Some(labels.join(", "))),
                                        _ => ("deny", None),
                                    };
                                    let mut body = serde_json::json!({ "action": action });
                                    if let Some(fb) = feedback {
                                        body["feedback"] = fb.into();
                                    }
                                    let _ = client_c
                                        .raw_post(
                                            &format!("/approvals/{question_id_c}/action"),
                                            &body,
                                        )
                                        .await;
                                });
                            }
                        }
                    }
                    "subagent_started" => {
                        let subagent_id =
                            msg.data["subagent_id"].as_str().unwrap_or("").to_string();
                        let mode = msg.data["mode"].as_str().unwrap_or("worker").to_string();
                        let task = msg.data["task"].as_str().unwrap_or("").to_string();
                        if !subagent_id.is_empty() {
                            let mut app = app_arc.lock();
                            let mut tracker = cade_tui::subagent_tracker::SubagentTracker::new(
                                subagent_id.clone(),
                                mode.clone(),
                            );
                            tracker.current_tool = Some("init".to_string());
                            if !task.is_empty() {
                                tracker.push_output(format!("[TASK]: {task}"));
                            }
                            app.subagent_trackers.push(tracker);
                            app.show_toast(
                                format!("Subagent [{mode}] started: {subagent_id}"),
                                crate::ui::ToastLevel::Info,
                            );
                            app.draw_dirty = true;
                            let _ = app.draw();
                        }
                    }
                    "subagent_output" => {
                        let subagent_id = msg.data["subagent_id"].as_str().unwrap_or("");
                        if let Some(chunk) = msg.data["chunk"].as_str() {
                            let mut app = app_arc.lock();
                            if let Some(t) = app
                                .subagent_trackers
                                .iter_mut()
                                .find(|t| t.task_id == subagent_id)
                            {
                                t.push_output(chunk.to_string());
                                app.draw_dirty = true;
                                let _ = app.draw();
                            }
                        }
                    }
                    "subagent_tool_start" => {
                        let subagent_id = msg.data["subagent_id"].as_str().unwrap_or("");
                        let tool = msg.data["tool"].as_str().unwrap_or("tool");
                        let mut app = app_arc.lock();
                        if let Some(t) = app
                            .subagent_trackers
                            .iter_mut()
                            .find(|t| t.task_id == subagent_id)
                        {
                            t.current_tool = Some(tool.to_string());
                            t.tool_calls += 1;
                            t.push_output(format!("▶ tool: {tool}"));
                            app.draw_dirty = true;
                            let _ = app.draw();
                        }
                    }
                    "subagent_tool_end" => {
                        let subagent_id = msg.data["subagent_id"].as_str().unwrap_or("");
                        let mut app = app_arc.lock();
                        if let Some(t) = app
                            .subagent_trackers
                            .iter_mut()
                            .find(|t| t.task_id == subagent_id)
                        {
                            t.current_tool = None;
                            app.draw_dirty = true;
                            let _ = app.draw();
                        }
                    }
                    "subagent_complete" => {
                        let subagent_id = msg.data["subagent_id"].as_str().unwrap_or("");
                        let is_error = msg.data["is_error"].as_bool().unwrap_or(false);
                        let result_preview = msg.data["result_preview"].as_str().unwrap_or("");
                        let mut app = app_arc.lock();
                        if let Some(t) = app
                            .subagent_trackers
                            .iter_mut()
                            .find(|t| t.task_id == subagent_id)
                        {
                            t.current_tool = None;
                            if is_error {
                                t.status = cade_tui::subagent_tracker::SubagentStatus::Failed {
                                    finished_at: std::time::Instant::now(),
                                    error: result_preview.to_string(),
                                };
                                app.show_toast(
                                    format!("Subagent {subagent_id} failed"),
                                    crate::ui::ToastLevel::Error,
                                );
                            } else {
                                if !result_preview.is_empty() {
                                    t.push_output(format!("[RESULT]: {result_preview}"));
                                }
                                t.status = cade_tui::subagent_tracker::SubagentStatus::Completed {
                                    finished_at: std::time::Instant::now(),
                                };
                                app.show_toast(
                                    format!("Subagent {subagent_id} completed"),
                                    crate::ui::ToastLevel::Success,
                                );
                            }
                            app.draw_dirty = true;
                            let _ = app.draw();
                        }
                    }
                    _ => {}
                }
            }
            // Channel closed — flush any pending reasoning if no assistant message arrived
            if in_reasoning {
                let _ = app_arc.lock().commit_reasoning();
            }
        });

        // -- Streaming call (network I/O — on_event never touches TuiApp)
        let agent_id = self.agent_id();
        let cancel = &self.cancel_turn;

        let conv_id = self.conversation_id();
        let conv_ref = conv_id.as_deref();

        let _ = (
            is_tool_return,
            tool_call_id,
            tool_name,
            tool_output,
            ephemeral,
        );
        let mode_str = self.permissions.mode().to_string();
        let messages = match self
            .client
            .start_run_cancellable_with_mode(
                &agent_id,
                input,
                conv_ref,
                Some(&mode_str),
                on_event,
                Some(cancel),
            )
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                ui_task.abort();
                return Ok(self.abort_stream_ui(error.to_string()));
            }
        };

        // -- Drain UI consumer — let it process any remaining queued messages
        // on_event held the sender; the streaming call above consumed it (closure
        // dropped when stream_message_cancellable returned).  The channel is now
        // closed, so ui_rx.recv() will return None after draining.
        let _ = ui_task.await;

        let finish_reason_value = finish_reason_arc.lock().clone();

        // Safety-net commit: ensure reasoning/streaming are flushed even if the
        // UI task missed the final messages (e.g. channel race on success path).
        {
            let mut app = self.app.lock();
            let _ = app.commit_reasoning();
            let _ = app.commit_streaming();
            app.notify_if_unfocused(
                cade_tui::app::notifier::AttentionCue::TurnFinished,
                "Turn Complete",
                "CADE finished the task turn",
            );
        }

        // Post-stream diagnostics: finish reason, truncation heuristics, context usage.
        {
            let text = self.last_assistant_text.lock().clone();
            let trimmed = text.trim_end();
            let looks_truncated = !trimmed.is_empty()
                && (trimmed.ends_with(':')
                || trimmed.ends_with("—")
                || trimmed.ends_with("...")
                || trimmed.ends_with('-')
                // Ends with a list-item prefix that was never followed by content
                || trimmed.ends_with("1.")
                || trimmed.ends_with("2.")
                || trimmed.ends_with("3."));

            let mut hints: Vec<String> = Vec::new();
            let mut suppress_truncation_hint = false;

            if let Some(reason) = finish_reason_value.as_deref()
                && let Some((msg, category)) = finish_reason_hint(reason)
            {
                if matches!(category, FinishReasonCategory::OutputLimit) {
                    suppress_truncation_hint = true;
                }
                hints.push(msg);
            }

            if looks_truncated && !suppress_truncation_hint {
                hints.push(
                    "⚠ Response may be incomplete — the model stopped generating. Try: /new for a fresh conversation, or rephrase your question.".to_string()
                );
            }

            let context_pct_opt = { self.app.lock().context_pct };
            if let Some(pct) = context_pct_opt
                && pct >= 95
            {
                hints.push(format!(
                        "⚠ Context window is {pct}% full — CADE summarized or trimmed older turns. Consider /new or ask for a shorter reply."
                    ));
            }
            for msg in hints {
                self.tui_dim(msg);
            }
        }

        // Save run_id + last seq_id for crash recovery / reconnect
        let saved_run_id = run_id_cell.lock().clone();
        let saved_seq_id = *seq_id_cell.lock();
        if saved_run_id.is_some() || saved_seq_id.is_some() {
            let mut s = self.session.lock();
            let _ = s.set_run(saved_run_id, saved_seq_id);
        }

        // Keep TUI session cost in sync with computed stats
        {
            let (total_cost, _) = self.session_stats.lock().compute_cost();
            let mut app = self.app.lock();
            app.session_cost_usd = total_cost;
            let _ = app.draw();
        }

        Ok(messages)
    }
}

use super::Repl;
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};
use crate::Result;
use crate::support::text::{FinishReasonCategory, finish_reason_hint};
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

/// Keep typed instructions distinct from option labels: arbitrary input must
/// never turn a denial into an approval by matching a displayed label.
fn approval_decision(answer: Option<cade_tui::question::QuestionAnswer>) -> serde_json::Value {
    use cade_tui::question::QuestionAnswer;
    match answer {
        Some(QuestionAnswer::Single(label)) if label == "Allow once" => {
            serde_json::json!({"action": "approve"})
        }
        Some(QuestionAnswer::Single(label)) if label == "Allow for this session" => {
            serde_json::json!({"action": "approve_session"})
        }
        Some(QuestionAnswer::Custom(instructions)) => {
            serde_json::json!({"action": "deny", "feedback": instructions})
        }
        _ => serde_json::json!({"action": "deny"}),
    }
}

async fn submit_dialog_answer(
    client: &cade_agent::agent::client::HttpTransport,
    id: &str,
    body: &serde_json::Value,
) -> Result<()> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.raw_post(&format!("/approvals/{id}/action"), body),
    )
    .await
    .map_err(|_| crate::Error::custom("Decision answer acknowledgement timed out"))?
    .map(|_| ())
    .map_err(|e| crate::Error::custom(e.to_string()))
}

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

/// Own remote dialogs for this stream, including cleanup on disconnect/abort.
struct PendingApprovalDialogs {
    app: std::sync::Arc<parking_lot::Mutex<cade_tui::TuiApp>>,
    ids: std::collections::HashSet<String>,
}

impl PendingApprovalDialogs {
    fn resolve(&mut self, id: &str) {
        if self.ids.remove(id) {
            self.app.lock().resolve_approval(id);
        }
    }
}

impl Drop for PendingApprovalDialogs {
    fn drop(&mut self) {
        let mut app = self.app.lock();
        for id in &self.ids {
            app.resolve_approval(id);
        }
    }
}

// The real channel consumer is also the presentation test seam. Network and
// decision handling remain independent of whether text is shown live.
async fn consume_presented_events(
    mut events: tokio::sync::mpsc::UnboundedReceiver<CadeMessage>,
    live: bool,
    reasoning: &parking_lot::Mutex<String>,
    assistant: &parking_lot::Mutex<String>,
    mut present: impl FnMut(CadeMessage),
) {
    // Store byte ranges into the retained turn text, not a second copy of every
    // token/event. Adjacent chunks of the same kind share one phase entry.
    let mut deferred: Vec<(bool, std::ops::Range<usize>)> = Vec::new();
    while let Some(message) = events.recv().await {
        let text = message
            .reasoning_text()
            .map(|text| (true, text))
            .or_else(|| message.assistant_text().map(|text| (false, text)));
        if let Some((is_reasoning, text)) = text {
            let range = {
                let mut retained = if is_reasoning {
                    reasoning.lock()
                } else {
                    assistant.lock()
                };
                let start = retained.len();
                retained.push_str(text);
                start..retained.len()
            };
            if !live {
                if !range.is_empty() {
                    if let Some((kind, previous)) = deferred.last_mut()
                        && *kind == is_reasoning
                    {
                        previous.end = range.end;
                    } else {
                        deferred.push((is_reasoning, range));
                    }
                }
                continue;
            }
        }
        present(message);
    }
    // Channel closure is shared by completion, failure and cancellation. Drain
    // the phases once before stream_turn performs its final TUI commit.
    for (is_reasoning, range) in deferred {
        let text = {
            let retained = if is_reasoning {
                reasoning.lock()
            } else {
                assistant.lock()
            };
            retained[range].to_owned()
        };
        let (message_type, field) = if is_reasoning {
            ("reasoning_message", "reasoning")
        } else {
            ("assistant_message", "content")
        };
        present(CadeMessage {
            id: None,
            message_type: Some(message_type.into()),
            data: serde_json::json!({field: text}),
        });
    }
}

fn present_turn_text(app: &mut cade_tui::TuiApp, message: &CadeMessage) {
    if let Some(text) = message.reasoning_text() {
        if !text.is_empty() && app.has_streaming() {
            let _ = app.commit_streaming();
        }
        app.push_reasoning_chunk(text);
    } else if let Some(text) = message.assistant_text()
        && !text.is_empty()
    {
        app.commit_reasoning_inner();
        let _ = app.push_streaming_chunk(text);
    }
}

/// Only transport-delivered journal sequences can become recovery cursors.
fn record_recovery_sequence(cursor: &parking_lot::Mutex<Option<i64>>, message: &CadeMessage) {
    if let Some(sequence) = message.seq_id() {
        *cursor.lock() = Some(sequence);
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
        let live_text = self
            .streaming_enabled
            .load(std::sync::atomic::Ordering::SeqCst);
        let options = self.execution_options().await?;
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
            // Persist acceptance as it is observed, including when subsequent
            // observation detaches. Never wait for a successful stream return.
            let known_run_id = run_id_cell2.lock().clone();
            if let Some(id) = msg.run_id()
                && known_run_id.as_deref() != Some(id)
            {
                *run_id_cell2.lock() = Some(id.to_owned());
                let _ = session_arc.lock().set_run(Some(id.to_owned()), None);
            }
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
            record_recovery_sequence(&seq_id_cell2, msg);
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
            let mut in_reasoning = false;
            let mut approvals = PendingApprovalDialogs {
                app: app_arc.clone(),
                ids: Default::default(),
            };
            let mut dialog_tasks = tokio::task::JoinSet::new();
            consume_presented_events(ui_rx, live_text, &reasoning_buf, &assistant_buf, |msg| {
                match msg.msg_type() {
                    "reasoning_message" => {
                        if msg.reasoning_text().is_some() {
                            in_reasoning = true;
                            present_turn_text(&mut app_arc.lock(), &msg);
                        }
                    }
                    "assistant_message" => {
                        if let Some(text) = msg.assistant_text() {
                            if !text.is_empty() {
                                in_reasoning = false;
                                let line_count = {
                                    let mut app = app_arc.lock();
                                    present_turn_text(&mut app, &msg);
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
                            None => return,
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
                    "approval_resolved" | "question_resolved" => {
                        if let Some(id) = msg.id.as_deref().or_else(|| msg.data["id"].as_str()) {
                            approvals.resolve(id);
                        }
                    }
                    "approval_required" => {
                        let Some(request) = msg.approval_request() else {
                            return;
                        };
                        let id = request.id.to_string();
                        let tool = request.tool_name.to_string();
                        let reason = request.reason.to_string();
                        let args_val = request.arguments;
                        let subagent = msg.data.get("subagent_id").and_then(|v| v.as_str());

                        let client_c = client_for_ui.clone();
                        let approval_id_c = id.clone();
                        let args_preview = serde_json::to_string_pretty(args_val)
                            .unwrap_or_else(|_| args_val.to_string());
                        let command_preview = args_val
                            .get("command")
                            .and_then(|value| value.as_str())
                            .map(|command| format!("\n\nCommand:\n```bash\n{command}\n```"))
                            .unwrap_or_default();

                        let question = cade_tui::question::Question {
                            header: match subagent {
                                Some(subagent_id) => {
                                    format!("Approve {tool} · Subagent {subagent_id}")
                                }
                                None => format!("Approve {tool}"),
                            },
                            text: format!(
                                "Approval {id}: allow '{tool}' to run?\n\nReason: {reason}{command_preview}\n\nArguments:\n```json\n{args_preview}\n```"
                            ),
                            options: vec![
                                cade_tui::question::QuestionOption {
                                    label: "Allow once".to_string(),
                                    description: "Approve this single tool execution".to_string(),
                                },
                                cade_tui::question::QuestionOption {
                                    label: "Allow for this session".to_string(),
                                    description: "Allow this tool for this working session, including Subagents"
                                        .to_string(),
                                },
                                cade_tui::question::QuestionOption {
                                    label: "Deny".to_string(),
                                    description: "Reject execution of this tool".to_string(),
                                },
                            ],
                            multi_select: false,
                            allow_other: true,
                            progress: None,
                        };

                        let rx_opt = {
                            if !approvals.ids.insert(id.clone()) {
                                return;
                            }
                            let mut app = app_arc.lock();
                            app.show_toast(
                                format!("🔒 Approval required for {tool}"),
                                crate::ui::ToastLevel::Warning,
                            );
                            app.ask_approval_async(id.clone(), question).ok()
                        };

                        if let Some(rx) = rx_opt {
                            let app_for_error = app_arc.clone();
                            dialog_tasks.spawn(async move {
                                // Closing the channel means the remote request was
                                // resolved or its stream ended, not a user denial.
                                let Ok(answer) = rx.await else {
                                    return;
                                };
                                let body = approval_decision(answer);
                                if let Err(error) =
                                    submit_dialog_answer(&client_c, &approval_id_c, &body).await
                                {
                                    let mut app = app_for_error.lock();
                                    app.show_toast(
                                        format!("Approval was not accepted: {error}"),
                                        crate::ui::ToastLevel::Error,
                                    );
                                    app.draw_dirty = true;
                                }
                            });
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
                            && let Some(arr) = questions_val.as_array()
                            && !arr.is_empty()
                        {
                            if !approvals.ids.insert(id.clone()) {
                                return;
                            }
                            let questions_list: Vec<cade_tui::question::Question> = arr
                                .iter()
                                .enumerate()
                                .map(|(idx, q_val)| {
                                    let header = q_val
                                        .get("header")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Question")
                                        .to_string();
                                    let text = q_val
                                        .get("question")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let multi_select = q_val
                                        .get("multiSelect")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    let options = q_val
                                        .get("options")
                                        .and_then(|v| v.as_array())
                                        .map(|opts| {
                                            opts.iter()
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

                                    cade_tui::question::Question {
                                        header,
                                        text,
                                        options,
                                        multi_select,
                                        allow_other: true,
                                        progress: if arr.len() > 1 {
                                            Some((idx + 1, arr.len()))
                                        } else {
                                            None
                                        },
                                    }
                                })
                                .collect();

                            let client_c = client_for_ui.clone();
                            let question_id_c = id.clone();
                            let app_arc_c = std::sync::Arc::clone(&app_arc);

                            dialog_tasks.spawn(async move {
                                let mut answers: std::collections::HashMap<String, String> =
                                    std::collections::HashMap::new();
                                let mut cancelled = false;

                                for q in questions_list {
                                    let header = q.header.clone();
                                    let rx_opt = {
                                        let mut app = app_arc_c.lock();
                                        app.show_toast(
                                            format!("❓ Question: {header}"),
                                            crate::ui::ToastLevel::Info,
                                        );
                                        app.ask_approval_async(question_id_c.clone(), q).ok()
                                    };

                                    let Some(rx) = rx_opt else {
                                        cancelled = true;
                                        break;
                                    };

                                    let answer_str = match rx.await {
                                        Ok(Some(cade_tui::question::QuestionAnswer::Single(
                                            ref label,
                                        ))) => label.clone(),
                                        Ok(Some(cade_tui::question::QuestionAnswer::Custom(
                                            ref answer,
                                        ))) => answer.clone(),
                                        Ok(Some(cade_tui::question::QuestionAnswer::Multi(
                                            ref labels,
                                        ))) => labels.join(", "),
                                        Err(_) => return, // remote resolution or observation ended
                                        _ => {
                                            cancelled = true;
                                            break;
                                        }
                                    };

                                    answers.insert(header, answer_str);
                                }

                                let body = if cancelled || answers.is_empty() {
                                    serde_json::json!({ "action": "deny" })
                                } else {
                                    let feedback = serde_json::to_string(&answers)
                                        .unwrap_or_else(|_| "".to_string());
                                    serde_json::json!({
                                        "action": "approve",
                                        "feedback": feedback,
                                    })
                                };
                                if let Err(error) = submit_dialog_answer(&client_c, &question_id_c, &body).await {
                                    let mut app = app_arc_c.lock();
                                    let message = format!("Answer for {question_id_c} was not accepted: {error}. Use /approvals to retry.");
                                    app.show_toast(message.clone(), crate::ui::ToastLevel::Error);
                                    let _ = app.push(RenderLine::ErrorMsg(message));
                                    app.draw_dirty = true;
                                }
                            });
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
                    "subagent_state" => {
                        let subagent_id = msg.data["subagent_id"].as_str().unwrap_or("");
                        let status = msg.data["status"].as_str().unwrap_or("unknown");
                        let mut app = app_arc.lock();
                        if let Some(t) = app
                            .subagent_trackers
                            .iter_mut()
                            .find(|t| t.task_id == subagent_id)
                        {
                            t.push_output(format!("[STATE]: {status}"));
                            app.draw_dirty = true;
                            let _ = app.draw();
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
            }).await;
            // Channel closed — flush any pending reasoning if no assistant message arrived
            if in_reasoning {
                let _ = app_arc.lock().commit_reasoning();
            }
        });

        // -- Streaming call (network I/O — on_event never touches TuiApp)
        let agent_id = self.agent_id();
        let cancel = &self.cancel_turn;

        // Conversation history remains independent of the Working Session's grants.
        if self.conversation_id().is_none() {
            let conversation = self.client.create_conversation(&agent_id, "").await?;
            let id = conversation["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| crate::Error::custom("Server did not return a conversation ID"))?;
            *self.conversation_id.lock() = Some(id.to_owned());
            self.session.lock().set_conversation(Some(id.to_owned()))?;
        }
        let conv_id = self.conversation_id();
        let conv_ref = conv_id.as_deref();

        let _ = (
            is_tool_return,
            tool_call_id,
            tool_name,
            tool_output,
            ephemeral,
        );
        let observation = self
            .client
            .start_run_cancellable_with_options(
                &agent_id,
                input,
                conv_ref,
                &options,
                on_event,
                Some(cancel),
            )
            .await;
        // Save the cursor for both completed and interrupted observation.
        let saved_run_id = run_id_cell.lock().clone();
        let saved_seq_id = *seq_id_cell.lock();
        if saved_run_id.is_some() {
            let _ = self.session.lock().set_run(saved_run_id, saved_seq_id);
        }
        let messages = match observation {
            Ok(messages) => messages,
            Err(error) => {
                // Drain partial output and retire dialogs before reporting the
                // observation error. Cleanup must never turn failure into Ok.
                let _ = ui_task.await;
                self.abort_stream_ui(error.to_string());
                return Err(crate::Error::custom(error.to_string()));
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
            if matches!(
                cade_agent::agent::client::RunOutcome::from_messages(&messages),
                Ok(cade_agent::agent::client::RunOutcome::Completed)
            ) {
                app.notify_if_unfocused(
                    cade_tui::app::notifier::AttentionCue::TurnFinished,
                    "Turn Complete",
                    "CADE finished the task turn",
                );
            }
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

#[cfg(test)]
mod approval_tests {
    use super::*;
    use cade_tui::question::QuestionAnswer;

    #[tokio::test]
    async fn cli_saved_cursor_recovers_text_before_gapped_finalization_failure() {
        use cade_agent::agent::{client::HttpTransport, session::SessionStore};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let start =
                serde_json::json!({"message_type":"stream_start","run_id":"r-cli-gap","seq_id":0});
            let fatal = serde_json::json!({"message_type":"error","run_id":"r-cli-gap","seq_id":10,"code":"run_finalization_failed","terminal_status_persisted":false,"error":"disk full"});
            let journal: Vec<_> = (1..10).map(|seq| serde_json::json!({"message_type":"assistant_message","run_id":"r-cli-gap","seq_id":seq,"content":format!("{seq} ")})).chain(std::iter::once(fatal.clone())).collect();
            for request_index in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let events = if request_index == 0 {
                    assert_eq!(path, "/v1/agents/agent/run");
                    vec![start.clone(), fatal.clone()]
                } else {
                    assert!(path.starts_with("/v1/runs/r-cli-gap/stream?starting_after="));
                    let after: i64 = path
                        .split_once("starting_after=")
                        .unwrap()
                        .1
                        .parse()
                        .unwrap();
                    assert_eq!(after, if request_index == 1 { 0 } else { 9 });
                    journal
                        .iter()
                        .filter(|event| event["seq_id"].as_i64().unwrap() > after)
                        .cloned()
                        .collect()
                };
                let body = events
                    .into_iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect::<String>();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let client = HttpTransport::new(format!("http://{address}"), String::new()).unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut store = SessionStore::load(workspace.path());
        let cursor = parking_lot::Mutex::new(None);
        let seen = parking_lot::Mutex::new(Vec::<CadeMessage>::new());
        let record = |event: &CadeMessage| {
            record_recovery_sequence(&cursor, event); // same callback bookkeeping as stream_turn
            seen.lock().push(event.clone());
        };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.start_run("agent", "hello", None, &record),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains("incomplete"));
        // The actual stream error path persists this cursor before returning Err.
        store
            .set_run(Some("r-cli-gap".into()), *cursor.lock())
            .unwrap();
        let saved = SessionStore::load(workspace.path()).session;
        assert_eq!(
            saved.last_seq_id,
            Some(0),
            "CLI must not persist the diagnostic's unvalidated sequence"
        );
        for expected_text in ["1 2 3 4 5 6 7 8 9 ", ""] {
            let saved = SessionStore::load(workspace.path()).session;
            seen.lock().clear();
            let error = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                client.resume_run(
                    saved.run_id.as_deref().unwrap(),
                    saved.last_seq_id.unwrap(),
                    &record,
                ),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(error.to_string().contains("disk full"));
            assert_eq!(
                seen.lock()
                    .iter()
                    .filter_map(|event| event.assistant_text())
                    .collect::<String>(),
                expected_text
            );
            assert_eq!(seen.lock().last().unwrap().seq_id(), None);
            assert_eq!(seen.lock().last().unwrap().msg_type(), "error");
            store.set_run(saved.run_id, *cursor.lock()).unwrap();
            assert_eq!(
                SessionStore::load(workspace.path()).session.last_seq_id,
                Some(9)
            );
        }
        peer.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires tty; exercised with script(1)"]
    async fn buffered_presentation_preserves_actual_tui_phase_order_and_modal_input() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut app = cade_tui::TuiApp::new(
            cade_core::permissions::PermissionMode::Default,
            "test".into(),
            "test".into(),
            None,
        );
        for live in [false, true] {
            app.lines.clear();
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            for (kind, field, text) in [
                ("reasoning_message", "reasoning", "first thought"),
                ("assistant_message", "content", "first answer"),
                ("reasoning_message", "reasoning", "second thought"),
                ("assistant_message", "content", "second answer"),
            ] {
                tx.send(CadeMessage {
                    id: None,
                    message_type: Some(kind.into()),
                    data: serde_json::json!({field:text}),
                })
                .unwrap();
            }
            drop(tx);
            consume_presented_events(
                rx,
                live,
                &parking_lot::Mutex::new(String::new()),
                &parking_lot::Mutex::new(String::new()),
                |event| present_turn_text(&mut app, &event),
            )
            .await;
            let _ = app.commit_reasoning();
            let _ = app.commit_streaming();
            let actual: Vec<_> = app
                .lines
                .iter()
                .filter_map(|line| match line {
                    RenderLine::AssistantText(text) => Some(("answer", text.as_str())),
                    RenderLine::Reasoning { content, .. } => Some(("thought", content.as_str())),
                    _ => None,
                })
                .collect();
            assert_eq!(
                actual,
                [
                    ("thought", "first thought"),
                    ("answer", "first answer"),
                    ("thought", "second thought"),
                    ("answer", "second answer")
                ]
            );
        }
        app.editor.set_text("/exit".into());
        let _answer = app
            .ask_approval_async(
                "ap-modal".into(),
                cade_tui::question::Question {
                    header: "Permission".into(),
                    text: "Allow?".into(),
                    options: vec![],
                    multi_select: false,
                    allow_other: true,
                    progress: None,
                },
            )
            .unwrap();
        let (owned, action) = app
            .dispatch_overlay_event(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert!(owned, "modal Enter must not reach busy command admission");
        assert!(action.is_none());
        assert_eq!(app.editor.text(), "/exit");
    }

    #[tokio::test]
    async fn presentation_consumer_buffers_only_text_and_flushes_each_turn_once() {
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for status in ["done", "error", "cancelled", "detached"] {
            // Off followed by on: each new turn uses its own accepted policy.
            for live in [false, true] {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let seen = Arc::new(parking_lot::Mutex::new(Vec::<CadeMessage>::new()));
                let control_seen = Arc::new(tokio::sync::Notify::new());
                let reasoning = Arc::new(parking_lot::Mutex::new(String::new()));
                let assistant = Arc::new(parking_lot::Mutex::new(String::new()));
                let task = {
                    let (seen, signal, reasoning, assistant) = (
                        seen.clone(),
                        control_seen.clone(),
                        reasoning.clone(),
                        assistant.clone(),
                    );
                    tokio::spawn(async move {
                        consume_presented_events(rx, live, &reasoning, &assistant, |message| {
                            let control = message.msg_type() == "question_required";
                            seen.lock().push(message);
                            if control {
                                signal.notify_one();
                            }
                        })
                        .await;
                    })
                };
                let events = [
                    serde_json::json!({"message_type":"reasoning_message","reasoning":"think α"}),
                    serde_json::json!({"message_type":"assistant_message","content":"first "}),
                    serde_json::json!({"message_type":"assistant_message","content":"answer"}),
                    serde_json::json!({"message_type":"reasoning_message","reasoning":"then β"}),
                    serde_json::json!({"message_type":"tool_progress_message"}),
                    serde_json::json!({"message_type":"approval_required"}),
                    serde_json::json!({"message_type":"question_required"}),
                ];
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
                let peer = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = [0; 8192];
                    let _ = socket.read(&mut request).await.unwrap();
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
                    for (seq, mut event) in events.into_iter().enumerate() {
                        event["run_id"] = "r-present".into();
                        event["seq_id"] = seq.into();
                        socket
                            .write_all(format!("data: {event}\n\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    finish_rx.await.unwrap();
                    let ending = if status == "detached" {
                        // A protocol failure ends the actual observer with Err;
                        // already received partial output still has to flush.
                        "data: {invalid-json}\n\n".into()
                    } else {
                        format!(
                            "data: {{\"message_type\":\"run_done\",\"run_id\":\"r-present\",\"seq_id\":7,\"status\":\"{status}\"}}\n\n"
                        )
                    };
                    socket.write_all(ending.as_bytes()).await.unwrap();
                });
                let client = cade_agent::agent::client::HttpTransport::new(
                    format!("http://{address}"),
                    String::new(),
                )
                .unwrap();
                let run = tokio::spawn(async move {
                    client
                        .start_run("agent", "hello", None, move |event| {
                            let _ = tx.send(event.clone());
                        })
                        .await
                });
                tokio::time::timeout(std::time::Duration::from_secs(1), control_seen.notified())
                    .await
                    .unwrap();
                assert_eq!(
                    seen.lock()
                        .iter()
                        .filter(|m| m.assistant_text().is_some() || m.reasoning_text().is_some())
                        .count()
                        > 0,
                    live,
                    "text leaked before end of {status} turn"
                );
                assert!(
                    seen.lock()
                        .iter()
                        .any(|m| m.msg_type() == "approval_required")
                );
                assert!(
                    seen.lock()
                        .iter()
                        .any(|m| m.msg_type() == "tool_progress_message")
                );
                finish_tx.send(()).unwrap();
                let result = tokio::time::timeout(std::time::Duration::from_secs(2), run)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(result.is_err(), status == "detached");
                task.await.unwrap();
                peer.await.unwrap();
                let output = seen
                    .lock()
                    .iter()
                    .filter_map(|m| {
                        m.assistant_text()
                            .map(|t| ("assistant", t.to_owned()))
                            .or_else(|| m.reasoning_text().map(|t| ("reasoning", t.to_owned())))
                    })
                    .collect::<Vec<_>>();
                if !live {
                    assert_eq!(
                        output.len(),
                        3,
                        "deferred chunks should coalesce into text phases"
                    );
                }
                let mut phases: Vec<(&str, String)> = Vec::new();
                for (kind, text) in output {
                    if let Some((last, content)) = phases.last_mut()
                        && *last == kind
                    {
                        content.push_str(&text);
                    } else {
                        phases.push((kind, text));
                    }
                }
                assert_eq!(
                    phases,
                    vec![
                        ("reasoning", "think α".into()),
                        ("assistant", "first answer".into()),
                        ("reasoning", "then β".into())
                    ]
                );
                assert_eq!(&*assistant.lock(), "first answer");
                assert_eq!(&*reasoning.lock(), "think αthen β");
            }
        }
    }

    #[tokio::test]
    async fn decision_answer_http_rejection_is_returned_for_presentation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(
                String::from_utf8_lossy(&buf[..n]).starts_with("POST /v1/approvals/q-test/action ")
            );
            let body = "{\"detail\":\"answer rejected\"}";
            socket.write_all(format!("HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let client = cade_agent::agent::client::HttpTransport::new(
            format!("http://{address}"),
            String::new(),
        )
        .unwrap();
        let error = submit_dialog_answer(
            &client,
            "q-test",
            &serde_json::json!({"action":"approve", "feedback":"answer"}),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("answer rejected"), "{error}");
        peer.await.unwrap();
    }

    #[test]
    fn approval_choices_never_interpret_instructions_as_labels() {
        assert_eq!(
            approval_decision(Some(QuestionAnswer::Single("Allow once".into())))["action"],
            "approve"
        );
        assert_eq!(
            approval_decision(Some(QuestionAnswer::Single(
                "Allow for this session".into()
            )))["action"],
            "approve_session"
        );
        assert_eq!(
            approval_decision(Some(QuestionAnswer::Single("Deny".into())))["action"],
            "deny"
        );
        assert_eq!(approval_decision(None)["action"], "deny");
        assert_eq!(
            approval_decision(Some(QuestionAnswer::Custom("Allow once".into()))),
            serde_json::json!({"action":"deny", "feedback":"Allow once"})
        );
    }
}

use super::Repl;
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};
use crate::Result;
use crate::support::text::{FinishReasonCategory, finish_reason_hint};
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

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
        tool_output: &str,
        // When true, user message is sent to LLM but NOT persisted to DB.
        // Used for system-injected re-prompts (EMPTY_YIELD_REPROMPT) so they
        // don't pollute conversation history or consume future context window.
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
                        { let mut s = session_arc.lock();
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
                    { let mut stats = sess_stats.lock();
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
        let ui_task = tokio::spawn(async move {
            let mut ui_rx = ui_rx;
            let mut in_reasoning = false;
            let mut in_assistant = false;
            while let Some(msg) = ui_rx.recv().await {
                match msg.msg_type() {
                    "reasoning_message" => {
                        if let Some(text) = msg.reasoning_text() {
                            in_reasoning = true;
                            reasoning_buf.lock().push_str(text);
                            app_arc
                                .lock()
                                .push_reasoning_chunk(text);
                        }
                    }
                    "assistant_message" => {
                        if let Some(text) = msg.assistant_text() {
                            assistant_buf.lock().push_str(text);
                            if !text.is_empty() {
                                in_reasoning = false;
                                in_assistant = true;
                                let line_count = {
                                    let mut app = app_arc.lock();
                                    app.commit_reasoning_inner();
                                    let _ = app.push_streaming_chunk(text);
                                    app.lines.len()
                                };
                                if let Some(bar) = &bar_text_arc {
                                    let cur = bar.lock().clone();
                                    if !cur.starts_with("●") {
                                        *bar.lock() =
                                            format!("generating… ({line_count} lines)");
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
                        {
                            let mut app = app_arc.lock();
                            app.commit_reasoning_inner();
                            let _ = app.commit_streaming();
                        }
                        in_assistant = false;
                        if let Some(bar) = &bar_text_arc {
                            let tool_name = msg.data["tool_calls"][0]["function"]["name"]
                                .as_str()
                                .unwrap_or("tool");
                            let display = if let Some(pos) = tool_name.rfind("__") {
                                &tool_name[pos + 2..]
                            } else {
                                tool_name
                            };
                            *bar.lock() = format!("● {}…", display);
                        }
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
                    }
                    _ => {}
                }
            }
            // Channel closed — suppress unused-variable warnings.
            let _ = (in_reasoning, in_assistant);
        });

        // -- Streaming call (network I/O — on_event never touches TuiApp)
        let agent_id = self.agent_id();
        let cancel = &self.cancel_turn;

        fn is_cancel(e: &cade_agent::Error) -> bool {
            matches!(e, cade_agent::Error::Custom(s) if s == "__cancelled__")
        }

        let conv_id = self.conversation_id();
        let conv_ref = conv_id.as_deref();

        let messages = if is_tool_return {
            let reasoning_effort = self.reasoning_effort.lock().clone();
            match self
                .client
                .stream_tool_return_cancellable(
                    &agent_id,
                    tool_call_id,
                    tool_output,
                    false,
                    conv_ref,
                    reasoning_effort.as_deref(),
                    on_event,
                    Some(cancel),
                )
                .await
            {
                Ok(m) => m,
                Err(e) if is_cancel(&e) => {
                    ui_task.abort();
                    return Ok(self.abort_stream_ui("Turn interrupted"));
                }
                Err(e) => {
                    ui_task.abort();
                    return Ok(self.abort_stream_ui(e.to_string()));
                }
            }
        } else {
            use std::sync::atomic::Ordering;
            let streaming = self.streaming_enabled.load(Ordering::SeqCst);
            if streaming {
                // Consume any pasted images on the first (non-tool-return) turn.
                // Subsequent turns (tool returns, follow-ups) carry no images.
                let turn_images = if !is_tool_return {
                    std::mem::take(&mut self.pending_turn_images)
                } else {
                    vec![]
                };
                let reasoning_effort = self.reasoning_effort.lock().clone();
                match self
                    .client
                    .stream_message_cancellable_with_images(
                        &agent_id,
                        input,
                        conv_ref,
                        ephemeral,
                        turn_images,
                        reasoning_effort.as_deref(),
                        on_event,
                        Some(cancel),
                    )
                    .await
                {
                    Ok(m) => m,
                    Err(e) if is_cancel(&e) => {
                        ui_task.abort();
                        return Ok(self.abort_stream_ui("Turn interrupted"));
                    }
                    Err(e) => {
                        ui_task.abort();
                        return Ok(self.abort_stream_ui(e.to_string()));
                    }
                }
            } else {
                // Non-streaming path — single HTTP request, print result at end.
                // UI task is unused; abort it immediately.
                ui_task.abort();
                let turn_images_ns = if !is_tool_return {
                    std::mem::take(&mut self.pending_turn_images)
                } else {
                    vec![]
                };
                match self
                    .client
                    .send_message_with_images(&agent_id, input, turn_images_ns, ephemeral)
                    .await
                {
                    Ok(msgs) => {
                        for msg in &msgs {
                            if let Some(text) = msg.assistant_text()
                                && !text.is_empty()
                            {
                                let _ = self
                                    .app
                                    .lock()
                                    .push_streaming_chunk(text);
                            }
                        }
                        let _ = self.app.lock().commit_streaming();
                        msgs
                    }
                    Err(e) => {
                        let _ = self
                            .app
                            .lock()
                            .push(RenderLine::ErrorMsg(e.to_string()));
                        return Ok(vec![]);
                    }
                }
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
        }

        // Post-stream diagnostics: finish reason, truncation heuristics, context usage.
        {
            let text = self
                .last_assistant_text
                .lock()
                .clone();
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
        if saved_run_id.is_some() || saved_seq_id.is_some()
        {
            let mut s = self.session.lock();
            let _ = s.set_run(saved_run_id, saved_seq_id);
        }

        Ok(messages)
    }

}

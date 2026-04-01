/// Per-model token breakdown accumulated during the session.
#[derive(Debug, Default, Clone)]
pub(crate) struct ModelStats {
    pub(crate) reqs: u32,
    pub(crate) input_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) cache_write_tokens: u64,
    pub(crate) output_tokens: u64,
}

/// All session-level statistics accumulated by the REPL.
/// Wrapped in `Arc<Mutex<...>>` so it can be updated from stream closures.
#[derive(Debug)]
pub(crate) struct SessionStats {
    pub(crate) started_at: std::time::Instant,
    /// Total milliseconds the agent was actively thinking / streaming.
    pub(crate) agent_active_ms: u64,
    /// Milliseconds spent waiting for LLM API responses.
    pub(crate) api_time_ms: u64,
    /// Milliseconds spent executing local tools.
    pub(crate) tool_time_ms: u64,
    /// Total tool calls dispatched.
    pub(crate) tool_calls_total: u32,
    /// Tool calls that completed without error.
    pub(crate) tool_calls_ok: u32,
    /// Tool calls that returned an error result.
    pub(crate) tool_calls_err: u32,
    /// Tool call results the user explicitly approved.
    pub(crate) approved: u32,
    /// Tool call results the user was asked to review (approved OR denied).
    pub(crate) reviewed: u32,
    /// Lines added across all file-write / patch tool calls this session.
    pub(crate) lines_added: i64,
    /// Lines removed across all file-write / patch tool calls this session.
    pub(crate) lines_removed: i64,
    /// Per-model breakdown (keyed by the full model string e.g. "gemini/gemini-2.5-pro").
    pub(crate) per_model: std::collections::HashMap<String, ModelStats>,
    pub(crate) registry: std::sync::Arc<cade_ai::ModelRegistry>,
}

impl SessionStats {
    pub(crate) fn new() -> Self {
        Self {
            started_at: std::time::Instant::now(),
            agent_active_ms: 0,
            api_time_ms: 0,
            tool_time_ms: 0,
            tool_calls_total: 0,
            tool_calls_ok: 0,
            tool_calls_err: 0,
            approved: 0,
            reviewed: 0,
            lines_added: 0,
            lines_removed: 0,
            per_model: std::collections::HashMap::new(),
            registry: std::sync::Arc::new(cade_ai::ModelRegistry::new()),
        }
    }

    /// Record a usage_statistics SSE event.
    pub(crate) fn record_usage(
        &mut self,
        model: &str,
        input: u64,
        cache_read: u64,
        cache_write: u64,
        output: u64,
    ) {
        let key = if model.is_empty() {
            "unknown".to_string()
        } else {
            model.to_string()
        };
        let e = self.per_model.entry(key).or_default();
        e.reqs += 1;
        e.input_tokens += input;
        e.cache_read_tokens += cache_read;
        e.cache_write_tokens += cache_write;
        e.output_tokens += output;
    }

    /// Compute total USD cost and per-model breakdown, sorted by cost descending.
    pub(crate) fn compute_cost(&self) -> (f64, Vec<(String, f64)>) {
        let mut total = 0.0f64;
        let mut by_model: Vec<(String, f64)> = Vec::new();
        for (model, ms) in &self.per_model {
            let p = self.registry.pricing_for_model(model);
            let cost = (ms.input_tokens as f64 * p.input) / 1_000_000.0
                + (ms.output_tokens as f64 * p.output) / 1_000_000.0
                + (ms.cache_read_tokens as f64 * p.cache_read) / 1_000_000.0
                + (ms.cache_write_tokens as f64 * p.cache_write) / 1_000_000.0;
            total += cost;
            by_model.push((model.clone(), cost));
        }
        by_model.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        (total, by_model)
    }

    /// Render a structured stats card as a Vec of RenderLine for the TUI.
    pub(crate) fn render_card(&self, auth_method: &str, session_id: &str) -> Vec<crate::ui::RenderLine> {
        use crate::ui::RenderLine;

        let wall_secs = self.started_at.elapsed().as_secs();
        let agent_secs = self.agent_active_ms / 1000;
        let api_secs = self.api_time_ms / 1000;
        let tool_secs = self.tool_time_ms / 1000;

        let fmt_dur = |s: u64| -> String {
            if s >= 3600 {
                format!("{}h {:02}m {:02}s", s / 3600, (s % 3600) / 60, s % 60)
            } else if s >= 60 {
                format!("{}m {:02}s", s / 60, s % 60)
            } else {
                format!("{}s", s)
            }
        };
        let fmt_tok = |n: u64| -> String {
            if n >= 1_000_000 {
                format!("{:.1}M", n as f64 / 1_000_000.0)
            } else if n >= 1_000 {
                format!("{:.1}K", n as f64 / 1_000.0)
            } else {
                n.to_string()
            }
        };

        let total_ok = self.tool_calls_ok;
        let total_err = self.tool_calls_err;
        let total = self.tool_calls_total;
        let success_pct = if total > 0 {
            100.0 * total_ok as f64 / total as f64
        } else {
            0.0
        };
        let agree_pct = if self.reviewed > 0 {
            100.0 * self.approved as f64 / self.reviewed as f64
        } else {
            100.0
        };

        let total_input: u64 = self.per_model.values().map(|m| m.input_tokens).sum();
        let total_cache: u64 = self.per_model.values().map(|m| m.cache_read_tokens).sum();
        let total_write: u64 = self.per_model.values().map(|m| m.cache_write_tokens).sum();
        let cache_pct = if total_input + total_cache > 0 {
            100.0 * total_cache as f64 / (total_input + total_cache) as f64
        } else {
            0.0
        };

        let mut out: Vec<RenderLine> = Vec::new();

        // -- Header
        out.push(RenderLine::InfoHeader("  ◆ Session Stats".to_string()));
        out.push(RenderLine::Blank);

        if !session_id.is_empty() {
            let id_disp = if session_id.len() > 20 {
                format!("{}…", &session_id[..20])
            } else {
                session_id.to_string()
            };
            out.push(RenderLine::Pair {
                label: "Session ID".to_string(),
                value: id_disp,
            });
        }
        if !auth_method.is_empty() {
            out.push(RenderLine::Pair {
                label: "Auth Method".to_string(),
                value: auth_method.to_string(),
            });
        }

        // -- Tool Calls
        out.push(RenderLine::Blank);
        out.push(RenderLine::InfoHeader("  Tool Calls".to_string()));
        out.push(RenderLine::Pair {
            label: "Total".to_string(),
            value: format!("{}  (✓ {}  ✗ {})", total, total_ok, total_err),
        });
        out.push(RenderLine::Pair {
            label: "Success Rate".to_string(),
            value: format!("{success_pct:.1}%"),
        });
        if self.reviewed > 0 {
            out.push(RenderLine::Pair {
                label: "User Approval".to_string(),
                value: format!("{agree_pct:.1}%  ({} reviewed)", self.reviewed),
            });
        }
        if self.lines_added != 0 || self.lines_removed != 0 {
            out.push(RenderLine::Pair {
                label: "Code Changes".to_string(),
                value: format!("+{}  −{}", self.lines_added, self.lines_removed.abs()),
            });
        }

        // -- Performance
        out.push(RenderLine::Blank);
        out.push(RenderLine::InfoHeader("  Performance".to_string()));
        out.push(RenderLine::Pair {
            label: "Wall Time".to_string(),
            value: fmt_dur(wall_secs),
        });
        out.push(RenderLine::Pair {
            label: "Agent Active".to_string(),
            value: fmt_dur(agent_secs),
        });
        if agent_secs > 0 {
            let api_p = 100.0 * api_secs as f64 / agent_secs as f64;
            let tool_p = 100.0 * tool_secs as f64 / agent_secs as f64;
            out.push(RenderLine::Pair {
                label: "  » API Time".to_string(),
                value: format!("{}  ({:.1}%)", fmt_dur(api_secs), api_p),
            });
            out.push(RenderLine::Pair {
                label: "  » Tool Time".to_string(),
                value: format!("{}  ({:.1}%)", fmt_dur(tool_secs), tool_p),
            });
        }

        // -- Model Usage table
        if !self.per_model.is_empty() {
            out.push(RenderLine::Blank);
            out.push(RenderLine::InfoHeader("  Model Usage".to_string()));

            let mut models: Vec<_> = self.per_model.iter().collect();
            models.sort_by(|a, b| b.1.reqs.cmp(&a.1.reqs));

            let headers = vec![
                "Model".to_string(),
                "Reqs".to_string(),
                "Input".to_string(),
                "Cache Read".to_string(),
                "Cache Write".to_string(),
                "Output".to_string(),
            ];
            let rows: Vec<Vec<String>> = models
                .iter()
                .map(|(model, ms)| {
                    let disp = if let Some(pos) = model.find('/') {
                        &model[pos + 1..]
                    } else {
                        model.as_str()
                    };
                    vec![
                        disp.to_string(),
                        ms.reqs.to_string(),
                        fmt_tok(ms.input_tokens),
                        fmt_tok(ms.cache_read_tokens),
                        fmt_tok(ms.cache_write_tokens),
                        fmt_tok(ms.output_tokens),
                    ]
                })
                .collect();

            out.push(RenderLine::Table { headers, rows });

            if total_cache > 0 {
                out.push(RenderLine::Pair {
                    label: "Cache Hit Rate".to_string(),
                    value: format!("{cache_pct:.1}% of input tokens served from cache"),
                });
            }
            if total_write > 0 {
                out.push(RenderLine::Pair {
                    label: "Cache Written".to_string(),
                    value: format!(
                        "{} tokens written to cache (billed at 1.25× input rate)",
                        fmt_tok(total_write)
                    ),
                });
            }
            out.push(RenderLine::DimMsg(
                "  /stats model  — per-model detail breakdown".to_string(),
            ));
        }

        out
    }

    /// Render a per-model detail table: rows = metrics, columns = models.
    pub(crate) fn render_model_detail(&self) -> Vec<crate::ui::RenderLine> {
        use crate::ui::RenderLine;

        if self.per_model.is_empty() {
            return vec![
                RenderLine::Blank,
                RenderLine::DimMsg("  No model usage recorded this session yet.".to_string()),
                RenderLine::Blank,
            ];
        }

        let fmt_tok = |n: u64| -> String {
            if n >= 1_000_000 {
                format!("{:.1}M", n as f64 / 1_000_000.0)
            } else if n >= 1_000 {
                format!("{:.1}K", n as f64 / 1_000.0)
            } else {
                n.to_string()
            }
        };

        // Sort models by total requests descending
        let mut models: Vec<(&String, &ModelStats)> = self.per_model.iter().collect();
        models.sort_by(|a, b| b.1.reqs.cmp(&a.1.reqs));

        // Column headers: blank label col + one col per model (strip provider prefix)
        let mut headers = vec!["Metric".to_string()];
        for (model, _) in &models {
            let disp = if let Some(pos) = model.find('/') {
                &model[pos + 1..]
            } else {
                model.as_str()
            };
            headers.push(disp.to_string());
        }

        // Build rows
        let metric_names = [
            "Requests",
            "Input",
            "Cache Read",
            "Cache Write",
            "Output",
            "Cache %",
        ];
        let mut rows: Vec<Vec<String>> = metric_names
            .iter()
            .map(|m| {
                let mut row = vec![m.to_string()];
                for (_, ms) in &models {
                    let val = match *m {
                        "Requests" => ms.reqs.to_string(),
                        "Input" => fmt_tok(ms.input_tokens),
                        "Cache Read" => fmt_tok(ms.cache_read_tokens),
                        "Cache Write" => fmt_tok(ms.cache_write_tokens),
                        "Output" => fmt_tok(ms.output_tokens),
                        "Cache %" => {
                            let total = ms.input_tokens + ms.cache_read_tokens;
                            if total > 0 {
                                format!(
                                    "{:.1}%",
                                    100.0 * ms.cache_read_tokens as f64 / total as f64
                                )
                            } else {
                                "—".to_string()
                            }
                        }
                        _ => "—".to_string(),
                    };
                    row.push(val);
                }
                row
            })
            .collect();

        // Totals row
        let total_reqs: u32 = models.iter().map(|(_, m)| m.reqs).sum();
        let total_in: u64 = models.iter().map(|(_, m)| m.input_tokens).sum();
        let total_cache: u64 = models.iter().map(|(_, m)| m.cache_read_tokens).sum();
        let total_write: u64 = models.iter().map(|(_, m)| m.cache_write_tokens).sum();
        let total_out: u64 = models.iter().map(|(_, m)| m.output_tokens).sum();
        let total_all = total_in + total_cache;
        let cache_pct_total = if total_all > 0 {
            format!("{:.1}%", 100.0 * total_cache as f64 / total_all as f64)
        } else {
            "—".to_string()
        };

        let mut totals_row = vec!["Total".to_string()];
        for (_, ms) in &models {
            let tot_in_model = ms.input_tokens + ms.cache_read_tokens;
            let cpct = if tot_in_model > 0 {
                format!(
                    "{:.1}%",
                    100.0 * ms.cache_read_tokens as f64 / tot_in_model as f64
                )
            } else {
                "—".to_string()
            };
            totals_row.push(format!(
                "{}r  {}i  {}cr  {}cw  {}o  {}",
                ms.reqs,
                fmt_tok(ms.input_tokens),
                fmt_tok(ms.cache_read_tokens),
                fmt_tok(ms.cache_write_tokens),
                fmt_tok(ms.output_tokens),
                cpct,
            ));
        }
        // For multi-model: add a grand-total column if >1 model
        if models.len() > 1 {
            rows[0].push(total_reqs.to_string());
            rows[1].push(fmt_tok(total_in));
            rows[2].push(fmt_tok(total_cache));
            rows[3].push(fmt_tok(total_write));
            rows[4].push(fmt_tok(total_out));
            rows[5].push(cache_pct_total);
            headers.push("Total".to_string());
        }

        vec![
            RenderLine::Blank,
            RenderLine::InfoHeader("  ◆ Model Usage Detail".to_string()),
            RenderLine::Blank,
            RenderLine::Table { headers, rows },
            RenderLine::Blank,
            RenderLine::DimMsg("  /stats        — full session card".to_string()),
            RenderLine::Blank,
        ]
    }
}

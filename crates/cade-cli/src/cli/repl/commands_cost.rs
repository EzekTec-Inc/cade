//! /cost command handler.

use crate::Result;
use super::stats::ModelStats;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_cost(
        &mut self,
    ) -> Result<bool> {
            let (total_cost, by_model) = {
                let stats = self.session_stats.lock();
                stats.compute_cost()
            };
            let (wall_ms, api_ms, lines_added, lines_removed) = {
                let stats = self.session_stats.lock();
                (
                    stats.started_at.elapsed().as_millis() as u64,
                    stats.agent_active_ms,
                    stats.lines_added,
                    stats.lines_removed,
                )
            };
            let per_model_snap: Vec<(String, ModelStats)> = {
                let stats = self.session_stats.lock();
                stats
                    .per_model
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            };
            let fmt_dur = |ms: u64| -> String {
                let s = ms / 1000;
                if s >= 3600 {
                    format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
                } else if s >= 60 {
                    format!("{}m {}s", s / 60, s % 60)
                } else {
                    format!("{}s", s)
                }
            };
            let fmt_tok = |n: u64| -> String {
                if n >= 1_000_000 {
                    format!("{:.1}M", n as f64 / 1_000_000.0)
                } else if n >= 1_000 {
                    format!("{:.1}k", n as f64 / 1_000.0)
                } else {
                    n.to_string()
                }
            };
            let mut lines: Vec<crate::ui::RenderLine> = vec![
                crate::ui::RenderLine::Blank,
                crate::ui::RenderLine::InfoHeader("  ◆ Session Cost".to_string()),
                crate::ui::RenderLine::Blank,
                crate::ui::RenderLine::Pair {
                    label: "Total cost".to_string(),
                    value: format!("${:.2}", total_cost),
                },
                crate::ui::RenderLine::Pair {
                    label: "Total duration (API)".to_string(),
                    value: fmt_dur(api_ms),
                },
                crate::ui::RenderLine::Pair {
                    label: "Total duration (wall)".to_string(),
                    value: fmt_dur(wall_ms),
                },
            ];
            if lines_added != 0 || lines_removed != 0 {
                lines.push(crate::ui::RenderLine::Pair {
                    label: "Total code changes".to_string(),
                    value: format!(
                        "{} lines added, {} lines removed",
                        lines_added,
                        lines_removed.abs()
                    ),
                });
            }
            if !by_model.is_empty() {
                lines.push(crate::ui::RenderLine::Blank);
                lines.push(crate::ui::RenderLine::DimMsg(
                    "  Usage by model:".to_string(),
                ));
                for (model, cost) in &by_model {
                    if let Some(ms) = per_model_snap
                        .iter()
                        .find(|(k, _)| k == model)
                        .map(|(_, v)| v)
                    {
                        let model_short =
                            model.rsplit('/').next().unwrap_or(model.as_str());
                        lines.push(crate::ui::RenderLine::DimMsg(format!(
                            "     {}   (${:.2})",
                            model_short, cost,
                        )));
                        let mut fields: Vec<String> = Vec::new();
                        if ms.input_tokens > 0 {
                            fields.push(format!("{} input", fmt_tok(ms.input_tokens)));
                        }
                        if ms.output_tokens > 0 {
                            fields
                                .push(format!("{} output", fmt_tok(ms.output_tokens)));
                        }
                        if ms.cache_read_tokens > 0 {
                            fields.push(format!(
                                "{} cache read",
                                fmt_tok(ms.cache_read_tokens)
                            ));
                        }
                        if ms.cache_write_tokens > 0 {
                            fields.push(format!(
                                "{} cache write",
                                fmt_tok(ms.cache_write_tokens)
                            ));
                        }
                        if !fields.is_empty() {
                            lines.push(crate::ui::RenderLine::DimMsg(format!(
                                "       {}",
                                fields.join(" · ")
                            )));
                        }
                    }
                }
            }
            lines.push(crate::ui::RenderLine::Blank);
            lines.push(crate::ui::RenderLine::DimMsg(
                "  Pricing estimates — check provider docs for current rates."
                    .to_string(),
            ));
            lines.push(crate::ui::RenderLine::Blank);
            let mut app = self.app.lock();
            for line in lines {
                let _ = app.push(line);
            }
        Ok(false)
    }
}

//! /context command handler.

use super::Repl;
use crate::Result;
use crate::ui::RenderLine;

impl Repl {
    pub(crate) async fn cmd_context(&mut self, _stdout: &mut std::io::Stdout) -> Result<bool> {
        let model = self.current_model.lock().clone();
        let window = cade_ai::catalogue::context_window_for_model(&model) as u64;
        let pct_opt = self.app.lock().context_pct;
        let agent_id = self.agent_id();
        let conv_id = self.conversation_id();
        // -- Per-category token estimates (indices match ContextBar categories)
        // Cat 0: system prompt
        let sys_tok = self
            .client
            .get_agent(&agent_id)
            .await
            .ok()
            .and_then(|a| a.system_prompt)
            .map(|s| (s.chars().count() / 3) as u64)
            .unwrap_or(0);
        // Cat 1: native tool schemas
        // Cat 2: MCP tool schemas
        let mcp_schemas = self.mcp.all_tool_schemas().await;
        let mcp_tok = (mcp_schemas
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .map(|s| s.len())
            .sum::<usize>()
            / 3) as u64;
        // Cat 3: memory blocks
        let mem_blocks = self.client.get_memory(&agent_id).await.unwrap_or_default();
        let mem_tok = (mem_blocks
            .iter()
            .map(|b| b.value.chars().count())
            .sum::<usize>()
            / 3) as u64;
        // Cat 4: skills loaded this session
        let skills_tok = {
            let skills = self.skills.lock();
            (skills.iter().map(|s| s.body.chars().count()).sum::<usize>() / 3) as u64
        };
        // Cat 5: conversation messages
        let msgs = self
            .client
            .get_conversation_messages(&agent_id, conv_id.as_deref().unwrap_or(""))
            .await
            .unwrap_or_default();
        let msg_tok = (msgs
            .iter()
            .map(|m| {
                m.get("char_count")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or_else(|| {
                        // Fallback if char_count is missing or content is string
                        m["content"].as_str().map(|s| s.len()).unwrap_or(0)
                    })
            })
            .sum::<usize>()
            / 3) as u64;
        // Cat 1: native tools residual (server pct - known categories)
        let known_excl_tools = sys_tok + mcp_tok + mem_tok + skills_tok + msg_tok;
        let tools_tok = pct_opt
            .map(|p| (p as u64 * window / 100).saturating_sub(known_excl_tools))
            .unwrap_or(0);
        let total_used = known_excl_tools + tools_tok;
        // Cat 6: free
        let buffer_tok = window * 3 / 100;
        let free_tok = window.saturating_sub(total_used + buffer_tok);
        // Cat 7: autocompact buffer
        let pct_val = pct_opt
            .unwrap_or_else(|| (total_used * 100).checked_div(window).unwrap_or(0).min(100) as u8);
        let model_short = model.rsplit('/').next().unwrap_or(&model).to_string();
        // Emit single ContextBar entry
        let category_tokens = vec![
            sys_tok,    // 0 system
            tools_tok,  // 1 tools
            mcp_tok,    // 2 mcp
            mem_tok,    // 3 memory
            skills_tok, // 4 skills
            msg_tok,    // 5 messages
            free_tok,   // 6 free
            buffer_tok, // 7 buffer
        ];
        {
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::Blank);
            let _ = app.push(RenderLine::ContextBar {
                model: model_short,
                window,
                pct: pct_val,
                category_tokens,
            });
        }
        // -- Detail sections below the bar
        // MCP tools
        let mcp_fmt = |n: u64| -> String {
            if n >= 1_000 {
                format!("{:.1}k", n as f64 / 1_000.0)
            } else {
                n.to_string()
            }
        };
        {
            let mcp_statuses = self.mcp.status().await;
            let loaded: Vec<_> = mcp_statuses.iter().filter(|s| !s.disabled).collect();
            let disabled: Vec<_> = mcp_statuses.iter().filter(|s| s.disabled).collect();
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::InfoHeader(format!(
                "  MCP Tools  ·  /mcp  (~{} tokens)",
                mcp_fmt(mcp_tok)
            )));
            if loaded.is_empty() {
                let _ = app.push(RenderLine::DimMsg(
                    "  (no MCP servers connected)".to_string(),
                ));
            } else {
                for s in &loaded {
                    let preview: String = {
                        let names: Vec<&str> = s
                            .tools
                            .iter()
                            .map(|t| t.rfind("__").map(|p| &t[p + 2..]).unwrap_or(t.as_str()))
                            .collect();
                        let p = names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
                        if names.len() > 5 {
                            format!("{}  +{} more", p, names.len() - 5)
                        } else {
                            p
                        }
                    };
                    let _ = app.push(RenderLine::DimMsg(format!("  └ {}:  {}", s.key, preview)));
                }
            }
            if !disabled.is_empty() {
                let _ = app.push(RenderLine::DimMsg("  Disabled".to_string()));
                for s in &disabled {
                    let _ = app.push(RenderLine::DimMsg(format!(
                        "  └ {}  (reconnect failed)",
                        s.key
                    )));
                }
            }
        }
        // Memory blocks
        {
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::Blank);
            let _ = app.push(RenderLine::InfoHeader(format!(
                "  Memory  ·  /memory  (~{} tokens)",
                mcp_fmt(mem_tok)
            )));
            if mem_blocks.is_empty() {
                let _ = app.push(RenderLine::DimMsg("  (no memory blocks)".to_string()));
            } else {
                for b in &mem_blocks {
                    let tok = (b.value.chars().count() / 3) as u64;
                    let desc = b.description.as_deref().unwrap_or("");
                    let suffix = if desc.is_empty() {
                        String::new()
                    } else {
                        format!("  —  {desc}")
                    };
                    let _ = app.push(RenderLine::DimMsg(format!(
                        "  └ {}:  ~{} tokens{}",
                        b.label,
                        mcp_fmt(tok),
                        suffix
                    )));
                }
            }
        }
        // Skills
        {
            let skills_snap = self.skills.lock().clone();
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::Blank);
            let _ = app.push(RenderLine::InfoHeader(format!(
                "  Skills  ·  /skills  (~{} tokens)",
                mcp_fmt(skills_tok)
            )));
            if skills_snap.is_empty() {
                let _ = app.push(RenderLine::DimMsg("  (no skills loaded)".to_string()));
            } else {
                for s in &skills_snap {
                    let tok = (s.body.chars().count() / 3) as u64;
                    let _ = app.push(RenderLine::DimMsg(format!(
                        "  └ {}  —  {}  (~{} tokens)",
                        s.id,
                        s.description,
                        mcp_fmt(tok)
                    )));
                }
            }
        }
        {
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::Blank);
            let _ = app.push(RenderLine::DimMsg(
                "  /stats  session totals  ·  /stats model  per-model breakdown".to_string(),
            ));
            let _ = app.push(RenderLine::Blank);
        }
        // Server-side live context accounting
        if let Ok(stats) = self
            .client
            .get_context_stats(&agent_id, conv_id.as_deref())
            .await
        {
            let t_inc = stats["turns_included"].as_u64().unwrap_or(0);
            let t_tot = stats["turns_total"].as_u64().unwrap_or(0);
            let t_omit = stats["turns_omitted"].as_u64().unwrap_or(0);
            let c_used = stats["chars_used"].as_u64().unwrap_or(0);
            let c_bud = stats["message_budget_chars"].as_u64().unwrap_or(0);
            let consol = stats["needs_consolidation"].as_bool().unwrap_or(false);
            let pct_c = if c_bud > 0 {
                format!("{:.0}%", 100.0 * c_used as f64 / c_bud as f64)
            } else {
                "?".to_string()
            };
            let mut app = self.app.lock();
            let _ = app.push(RenderLine::InfoHeader(
                "  ◆ Server Context Accounting (live)".to_string(),
            ));
            let _ = app.push(RenderLine::Blank);
            let turns_line = if t_omit > 0 {
                format!(
                    "  Turns:   {t_inc} of {t_tot} included  \
                         ({t_omit} omitted — use conversation_search to recover)"
                )
            } else {
                format!("  Turns:   {t_inc} of {t_tot} included  (none omitted)")
            };
            let _ = app.push(RenderLine::DimMsg(turns_line));
            let _ = app.push(RenderLine::DimMsg(format!(
                "  History: {c_used} / {c_bud} chars used  ({pct_c})"
            )));
            let consol_str = if consol {
                "yes — Sleeptime will summarise dropped turns after 60 s idle"
            } else {
                "none pending"
            };
            let _ = app.push(RenderLine::DimMsg(format!("  Consolidation: {consol_str}")));
            let _ = app.push(RenderLine::Blank);
        }
        Ok(false)
    }
}

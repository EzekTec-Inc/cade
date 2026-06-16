//! /search command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_search(&mut self, query: String) -> Result<bool> {
        if query.is_empty() {
            self.tui_dim("  Usage: /search <query>");
            return Ok(false);
        }
        // Run both searches concurrently. CLI `/search` always spans all
        // conversations for the agent (None) — agents can scope narrower
        // via the `conversation_search` tool with `conversation_id`.
        let agent_id = self.agent_id();
        let (msg_res, mem_res) = tokio::join!(
            self.client.search_messages(&agent_id, &query, None),
            self.client.search_memory(&agent_id, &query),
        );
        let msgs_empty = msg_res.as_ref().map(|v| v.is_empty()).unwrap_or(true);
        let mem_empty = mem_res.as_ref().map(|v| v.is_empty()).unwrap_or(true);
        if msgs_empty && mem_empty && msg_res.is_ok() && mem_res.is_ok() {
            self.tui_dim(format!("  No results for '{query}'"));
        } else {
            self.tui_blank();
            self.tui_hdr(format!("  Search results for '{query}'"));
            self.tui_blank();
            // Message results (FTS5 BM25-ranked)
            match &msg_res {
                Ok(msgs) if !msgs.is_empty() => {
                    self.tui_dim(format!("  ── Messages ({} match(es)) ──", msgs.len()));
                    for m in msgs.iter().take(8) {
                        let role = m["role"].as_str().unwrap_or("?");
                        let snippet = m["snippet"].as_str().unwrap_or("").trim();
                        let display = if snippet.is_empty() {
                            m["content"]["content"]
                                .as_str()
                                .or_else(|| m["content"].as_str())
                                .unwrap_or("")
                                .chars()
                                .take(100)
                                .collect::<String>()
                        } else {
                            snippet.chars().take(120).collect::<String>()
                        };
                        let score = m["score"].as_f64().unwrap_or(0.0);
                        self.tui_dim(format!("  [{role}] (bm25 {score:.2})  {display}"));
                    }
                    self.tui_blank();
                }
                Err(e) => self.tui_err(format!("  Message search error: {e}")),
                _ => {}
            }
            // Memory results (LIKE search)
            match &mem_res {
                Ok(blocks) if !blocks.is_empty() => {
                    self.tui_dim(format!("  ── Memory ({} match(es)) ──", blocks.len()));
                    for b in blocks.iter().take(5) {
                        let label = b["label"].as_str().unwrap_or("?");
                        let snippet = b["snippet"].as_str().unwrap_or("").trim();
                        let display: String = snippet.chars().take(120).collect();
                        self.tui_dim(format!("  [{label}]  {display}"));
                    }
                    self.tui_blank();
                }
                Err(e) => self.tui_err(format!("  Memory search error: {e}")),
                _ => {}
            }
        }
        Ok(false)
    }
}

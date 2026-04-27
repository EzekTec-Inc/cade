use crate::server::{config::ServerConfig, rate_limit::RateLimiter};
use cade_ai::{LlmMessage, LlmProvider, LlmRouter};
use cade_core::skills::Skill;
use cade_store::sqlite::Db;
use serde_json::Value;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Capacity of the per-agent context-build LRU cache.  Defined once
/// here so production binaries, the consolidation worker, and every
/// test helper share a single source of truth instead of duplicating
/// the literal `NonZeroUsize::new(20).unwrap()` 14 times across the
/// crate (see `state::tests::context_cache_capacity_is_nonzero`).
pub const CONTEXT_CACHE_CAPACITY: NonZeroUsize =
    NonZeroUsize::new(20).expect("CONTEXT_CACHE_CAPACITY must be > 0; literal is non-zero");

/// Re-export so call-sites in api/ can do `use crate::server::state::McpManager`.
pub use cade_agent::mcp::McpManager;

/// Shared application state injected into every axum handler
/// Tracks activity and consolidation state per agent.
#[derive(Debug, Clone)]
pub struct AgentActivity {
    pub last_active_ts: i64,
    pub needs_consolidation: bool,
    pub conversation_id: Option<String>,
    /// Turn counter snapshot at the time the last eager consolidation was
    /// triggered for this agent. Used by `should_eager_consolidate` to
    /// rate-limit the eager path (M3): even if `needs_consolidation` remains
    /// set across many rapid turns, a fresh run fires only once per
    /// `EAGER_CONSOLIDATION_TURN_THRESHOLD` turns. `0` means "never".
    pub last_consolidation_turn: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentMetrics {
    pub tool_outputs_compacted: usize,
    pub consolidation_runs: usize,
    pub chars_summarised: usize,
    pub chars_produced: usize,
    pub inflation_guard_hits: usize,
    /// P2: cumulative token totals for this agent across the server's
    /// lifetime.  Includes cache_read / cache_write so cost dashboards and
    /// future server-side cost guardrails see the full Anthropic / Gemini
    /// caching picture (previously dropped).
    pub input_tokens_total: u64,
    pub output_tokens_total: u64,
    pub cache_read_tokens_total: u64,
    pub cache_write_tokens_total: u64,
}

impl AgentMetrics {
    /// Add a single `TokenUsage` chunk into the cumulative totals.
    /// All four fields are accumulated atomically; cache fields are no
    /// longer dropped on the floor as in the pre-P2 implementation.
    pub fn accumulate_usage(&mut self, u: &cade_ai::TokenUsage) {
        self.input_tokens_total = self
            .input_tokens_total
            .saturating_add(u.input_tokens as u64);
        self.output_tokens_total = self
            .output_tokens_total
            .saturating_add(u.output_tokens as u64);
        self.cache_read_tokens_total = self
            .cache_read_tokens_total
            .saturating_add(u.cache_read_tokens as u64);
        self.cache_write_tokens_total = self
            .cache_write_tokens_total
            .saturating_add(u.cache_write_tokens as u64);
    }

    /// P4: compute cumulative session cost in USD using the provided pricing
    /// table.  Mirrors the formula in `cade-cli/src/cli/repl/stats.rs:91-95`
    /// so server-side numbers match the CLI's `/cost` view exactly.
    ///
    /// Pricing is per 1M tokens.  Returns 0.0 when pricing is unknown
    /// (`pricing.input == 0.0`) so callers can safely treat "unknown
    /// model" as "no guardrail".
    pub fn compute_cost_usd(&self, pricing: &cade_ai::ModelPricing) -> f64 {
        const PER_M: f64 = 1_000_000.0;
        (self.input_tokens_total as f64) * pricing.input / PER_M
            + (self.output_tokens_total as f64) * pricing.output / PER_M
            + (self.cache_read_tokens_total as f64) * pricing.cache_read / PER_M
            + (self.cache_write_tokens_total as f64) * pricing.cache_write / PER_M
    }
}

/// Phase 4: per-request telemetry recorded at the end of every
/// `build_context` call.  Captures every input that controls how the
/// context fits into the model window so we can prove (a) which defence
/// layer fired, (b) how close to the budget we ended up, and (c)
/// regressions.  Exposed via `GET /v1/agents/:id/context_stats` for live
/// inspection by the GUI / TUI.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ContextTelemetry {
    /// Model id used for this build (resolved post-routing).
    pub model: String,
    /// Total context window in tokens for the chosen model.
    pub window_tokens: usize,
    /// Input budget (window − output reserve) in chars at the legacy 3:1 ratio.
    pub input_budget_chars: usize,
    /// System overhead deduction in chars (system prompt + memory + skills).
    pub system_overhead_chars: usize,
    /// System overhead in real BPE tokens (P2-1 anchor).
    pub system_tokens: usize,
    /// Final char budget reserved for history (input_budget − system_overhead − tool_reserve).
    pub message_budget_chars: usize,
    /// Total chars actually packed into the assembled history (sum across messages).
    pub history_chars: usize,
    /// Native BPE tokens packed into the assembled history (P4 Pt 2).
    /// `chars_for_tokens(history_tokens)` ≈ `history_chars` modulo
    /// per-model encoder differences.
    pub history_tokens: usize,
    /// Native BPE tokens for the entire assembled context (system + history).
    /// This is the closest single number to what the provider will charge
    /// for the request's input.
    pub total_tokens: usize,
    /// Number of complete turns selected.
    pub turns_selected: usize,
    /// Number of complete turns omitted because the budget exhausted.
    pub turns_omitted: usize,
    /// Number of leading system messages preserved (static + dynamic).
    pub system_msg_count: usize,
    /// Number of skill bodies injected at full fidelity (P2-3).
    pub skills_full: usize,
    /// Number of skill bodies downgraded to summary entries (P2-3).
    pub skills_summary: usize,
    /// True iff the assembled context fits inside the input budget — this
    /// is the canonical "did our defences work" signal.
    pub fits_budget: bool,
    /// Wall-clock time spent in build_context, in microseconds.
    pub build_micros: u64,
}

/// Result of a completed background subagent, waiting for injection
/// into the parent agent's next agentic loop iteration.
#[derive(Debug, Clone)]
pub struct SubagentResult {
    pub subagent_id: String,
    pub tool_call_id: String,
    pub task_preview: String,
    pub result: String,
    pub is_error: bool,
    pub elapsed_secs: u32,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub llm: Arc<dyn LlmProvider>,
    /// Router behind RwLock for hot-reload — /connect adds providers without restart
    pub llm_router: Arc<RwLock<LlmRouter>>,
    pub config: Arc<ServerConfig>,
    /// MCP manager — executes tool calls on behalf of the agentic loop.
    /// Populated at startup from merged settings; empty when no MCP servers are configured.
    pub mcp: Arc<McpManager>,
    /// Per-agent token-bucket rate limiter
    pub rate_limiter: RateLimiter,
    /// Per-agent system-prompt cache: key=agent_id, value=(hash, system_prompt_without_tool_rule).
    /// When memory blocks are unchanged the hash matches and we reuse the cached string, keeping
    /// the system-prompt prefix byte-identical across turns so OpenAI/Gemini implicit caches hit.
    pub memory_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, (u64, String)>>>,
    /// Tracks `(last_active_ts, needs_consolidation, conversation_id)` per agent.
    /// `needs_consolidation` is set by `build_context` whenever older turns are
    /// dropped from the context window — the Sleeptime background task picks it
    /// up after 20 s of inactivity and summarises the dropped turns. An eager
    /// turn-count path in `build_context` (see `should_eager_consolidate`)
    /// covers continuous sessions that never hit the idle timer.
    pub agent_activity: Arc<RwLock<std::collections::HashMap<String, AgentActivity>>>,
    /// Tracks lifetime context efficiency metrics per agent.
    pub agent_metrics: Arc<RwLock<std::collections::HashMap<String, AgentMetrics>>>,
    /// Phase 4: most-recent `ContextTelemetry` per agent, captured at the
    /// end of every successful `build_context` call.  Read-only by
    /// outside callers; the `/v1/agents/:id/context_stats` endpoint
    /// projects this map.
    pub agent_context_telemetry: Arc<RwLock<std::collections::HashMap<String, ContextTelemetry>>>,
    /// LRU cache for `build_context` outputs to avoid recomputing history loops.
    /// Key: `format!("{agent_id}:{conversation_id}")`
    /// Value: `(max_rowid, cached_context_tuple)`
    pub context_cache:
        Arc<std::sync::Mutex<lru::LruCache<String, (u64, (String, Vec<LlmMessage>, Vec<Value>))>>>,

    // ── Skills ──────────────────────────────────────────────────────────────
    /// All discovered skills (global + project). Populated at boot from
    /// `discover_all_skills()`. Immutable after boot unless reloaded.
    pub all_skills: Arc<RwLock<Vec<Skill>>>,
    /// Per-agent loaded (activated) skill IDs. Only these skills' bodies are
    /// injected into the system prompt during `build_context`.
    /// Key: agent_id, Value: set of skill IDs that have been loaded via
    /// `load_skill` tool or auto-trigger.
    pub agent_skills: Arc<RwLock<std::collections::HashMap<String, Vec<String>>>>,

    // ── Subagents ───────────────────────────────────────────────────────────
    /// Completed background subagent results waiting to be injected into the
    /// parent agent's next agentic loop iteration.
    /// Key: parent agent_id, Value: vec of completed results.
    pub pending_subagent_results:
        Arc<RwLock<std::collections::HashMap<String, Vec<SubagentResult>>>>,
    /// Semaphore limiting concurrent subagent LLM calls server-side.
    pub subagent_semaphore: Arc<tokio::sync::Semaphore>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_ai::TokenUsage;

    #[test]
    fn context_cache_capacity_is_twenty() {
        // Locks the documented capacity.  Bumping the value is a
        // deliberate decision; this test exists to prevent it from
        // drifting silently (e.g. via a typo'd literal at one of the
        // 14 prior duplication sites).
        assert_eq!(CONTEXT_CACHE_CAPACITY.get(), 20);
    }

    #[test]
    fn context_cache_capacity_is_nonzero() {
        // Const-evaluated at compile time; runtime check is belt-and-
        // suspenders against accidental change to a zero literal.
        assert!(CONTEXT_CACHE_CAPACITY.get() > 0);
    }

    #[test]
    fn accumulate_usage_sums_all_four_token_fields() {
        let mut m = AgentMetrics::default();
        m.accumulate_usage(&TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 1_000,
            cache_write_tokens: 200,
            model: "anthropic/claude-sonnet-4".into(),
        });
        m.accumulate_usage(&TokenUsage {
            input_tokens: 25,
            output_tokens: 10,
            cache_read_tokens: 500,
            cache_write_tokens: 0,
            model: "anthropic/claude-sonnet-4".into(),
        });

        assert_eq!(m.input_tokens_total, 125);
        assert_eq!(m.output_tokens_total, 60);
        // P2 fix: cache fields must accumulate, not be silently dropped.
        assert_eq!(m.cache_read_tokens_total, 1_500);
        assert_eq!(m.cache_write_tokens_total, 200);
    }

    #[test]
    fn accumulate_usage_saturates_on_overflow() {
        let mut m = AgentMetrics::default();
        m.input_tokens_total = u64::MAX - 5;
        m.accumulate_usage(&TokenUsage {
            input_tokens: 100,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: String::new(),
        });
        assert_eq!(m.input_tokens_total, u64::MAX);
    }

    #[test]
    fn accumulate_usage_zero_is_noop() {
        let mut m = AgentMetrics::default();
        m.accumulate_usage(&TokenUsage::default());
        assert_eq!(m.input_tokens_total, 0);
        assert_eq!(m.output_tokens_total, 0);
        assert_eq!(m.cache_read_tokens_total, 0);
        assert_eq!(m.cache_write_tokens_total, 0);
    }

    // ── P4: compute_cost_usd ───────────────────────────────────────────────

    #[test]
    fn compute_cost_usd_matches_cli_formula() {
        // Sonnet 4 pricing (per 1M tokens): in=3, out=15, cr=0.3, cw=3.75
        let pricing = cade_ai::ModelPricing {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        };
        let m = AgentMetrics {
            input_tokens_total: 1_000_000,
            output_tokens_total: 200_000,
            cache_read_tokens_total: 5_000_000,
            cache_write_tokens_total: 100_000,
            ..Default::default()
        };
        // 3 + 3 + 1.5 + 0.375 = 7.875
        let cost = m.compute_cost_usd(&pricing);
        assert!((cost - 7.875).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn compute_cost_usd_zero_pricing_returns_zero() {
        // Unknown model → Default pricing (all zeros) → no guardrail trigger.
        let m = AgentMetrics {
            input_tokens_total: 999_999_999,
            output_tokens_total: 999_999_999,
            cache_read_tokens_total: 999_999_999,
            cache_write_tokens_total: 999_999_999,
            ..Default::default()
        };
        let cost = m.compute_cost_usd(&cade_ai::ModelPricing::default());
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn compute_cost_usd_empty_metrics_zero() {
        let pricing = cade_ai::ModelPricing {
            input: 100.0,
            output: 100.0,
            cache_read: 100.0,
            cache_write: 100.0,
        };
        assert_eq!(AgentMetrics::default().compute_cost_usd(&pricing), 0.0);
    }
}

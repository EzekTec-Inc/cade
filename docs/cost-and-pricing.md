# Cost & Pricing

CADE tracks token usage per model and translates it to USD via a
JSON-driven pricing registry. Every cost number you see in the TUI
footer, `/cost`, and the dashboard comes from the same code path.

## Quick view

```bash
/context              # context-window usage %
/stats [model]        # per-model token totals
/usage                # cumulative session tokens
/cost                 # session cost breakdown ($)
/pricing              # show pricing rules
/pricing sync         # fetch latest from upstream registry
/pricing edit         # open in $EDITOR
```

The TUI footer continuously displays:

```
↑12.3k ↓4.5k R8.1k W0.5k $0.347 47.5%/200k (Safe)
 ↑in   ↓out  cache  cache cost  ctx-pct/window mode
       read  write
```

## How costs are computed

The `cade-ai::ModelRegistry` loads pricing from
`~/.cade/pricing.json` (with bundled defaults). A model's pricing has
four lanes:

| Lane | Charged for | Typical scale |
|---|---|---|
| `input` | New input tokens (uncached) | $1–$15 / 1M |
| `output` | Generated tokens | $5–$60 / 1M |
| `cache_read` | Tokens served from prompt cache | $0.10–$1 / 1M |
| `cache_write` | Tokens written to prompt cache (first time) | $1–$10 / 1M |

`AgentMetrics::compute_cost_usd` multiplies each total by its lane and
sums. Unknown models get a zeros pricing → guardrails won't trigger
spuriously.

## Cost guardrails (env vars)

All disabled by default — set the env var to opt in.

| Variable | Effect | Default |
|---|---|---|
| `CADE_MAX_SESSION_COST_USD` | Hard $-cap on the agentic loop. The loop aborts as soon as the cumulative cost crosses this value. | unset |
| `CADE_TOOL_TURN_MAX_TOKENS` | Output-token cap on tool-dispatch turns (turn index > 1). Saves spend on verbose models. | unset |
| `CADE_GEMINI_CACHE_TTL_SECS` | Adaptive Gemini cache TTL (60–86400 s). Tune to session shape. | 3600 |

Examples:

```bash
# Cap a session at $2 cumulative spend
export CADE_MAX_SESSION_COST_USD=2.00

# Force tool-dispatch turns to be terse
export CADE_TOOL_TURN_MAX_TOKENS=1024

# Long sessions → longer Gemini cache TTL
export CADE_GEMINI_CACHE_TTL_SECS=7200
```

## Optimisations baked in

These run for every session, no opt-in needed:

| Phase | What | Saving |
|---|---|---|
| P1 | `skills` block moved to `system_static` cache anchor | ≈ 90% input on 10–30 KB skill payload |
| P2 | Cache-read / cache-write tokens fully accounted in `AgentMetrics` | accurate cost reporting |
| P3 | Auto-cheapest **compaction model** per provider (Anthropic→Haiku, OpenAI→4o-mini, Gemini→Flash, OpenRouter→GLM-free) | 3.8–19× on consolidation calls |
| P5 | Unused non-pinned tool schemas can be compressed, and OpenAI requests are capped at the provider's 128-tool limit while preserving priority meta tools such as `load_skill` | avoids oversized tool payloads and OpenAI API rejections |
| P8 | `tool_executions.output_chars` column for per-call cost observability | DB cheap; replaces `LENGTH(output)` scans |

## Per-agent compaction model

Override the default cheap compaction model:

```bash
/compaction-model anthropic/claude-3-5-haiku-latest
/compaction-model                 # CLI: empty arg clears the override
```

The GUI palette accepts `/compaction-model <model>` but **does not**
support the empty-arg clear (no confirmation UI). Use the CLI to clear.

## Pricing registry format

```json
{
  "anthropic/claude-sonnet-4-5": {
    "input": 3.0,
    "output": 15.0,
    "cache_read": 0.3,
    "cache_write": 3.75
  },
  "openai/gpt-4o-mini": {
    "input": 0.15,
    "output": 0.60,
    "cache_read": 0.075,
    "cache_write": 0.0
  }
}
```

Units are **USD per 1 million tokens**. Reload with `/pricing sync` (pulls
from CADE's bundled upstream) or `/pricing edit` (opens in `$EDITOR`).

The OpenAI provider also normalizes newer OpenAI model families: GPT-5-style and
o-series models use the high output-token budget path, and OpenAI tool payloads
are capped at 128 tools to match provider limits while keeping priority meta
tools (including `load_skill` and core memory-writing tools like `update_memory`) 
as well as essential MCP tools (with `serena__`, `cade-rag__`, and `cade-ide-mcp__` prefixes) available.

## Inspecting historical cost

```sql
-- Top 10 most expensive tool calls in the last day
SELECT tool_name, output_chars, created_at
  FROM tool_executions
  WHERE created_at > strftime('%s', 'now', '-1 day')
  ORDER BY output_chars DESC
  LIMIT 10;
```

`output_chars` is computed at insert time using
`output.chars().count()` (Unicode-correct). Legacy rows backfilled with
`LENGTH(output)` (bytes — over-counts multibyte by 2–4×, accepted as
historical approximation).

## Cost-aware patterns

- **Use `worker` subagents on cheap models** for read-heavy
  exploration. Override per-call: `run_subagent(model="anthropic/claude-haiku-4-5", …)`.
- **Pin `compaction_model` to the cheapest model** that gives an
  acceptable summary. Haiku and 4o-mini are typically fine.
- **Disable streaming** (`/stream`) when running headless to skip the
  per-chunk SSE overhead — small but adds up.
- **Strip unused tools** — disable MCP servers you aren't using; ITS
  reranking handles the rest. See
  [intelligent-tool-selection.md](intelligent-tool-selection.md).

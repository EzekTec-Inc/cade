# Intelligent Tool Selection (ITS)

CADE includes a built-in **intelligent tool selection** system that dynamically
filters the tools sent to the LLM on each request.  Instead of injecting every
registered tool schema (native + MCP) into the prompt — which can consume
10,000–20,000+ tokens — ITS reranks tools against the user's latest message and
keeps only the most relevant subset.

## Quick Start

Set two environment variables before starting `cade-server`:

```bash
export CADE_RERANKER_ENABLED=true   # turn it on
export CADE_RERANKER_TOP_N=15       # keep top 15 tools per request (default)
```

That's it.  On first use the local ONNX model (`ms-marco-MiniLM-L-6-v2`,
~90 MB) is automatically downloaded to `~/.cache/cade/models/reranker/`.

## How It Works

```
User message
    │
    ▼
┌──────────────────────────────┐
│  Collect all tool schemas    │  native + MCP  (e.g. 90 tools)
└──────────┬───────────────────┘
           │
           ▼
┌──────────────────────────────┐
│  Separate protected tools    │  bash, read_file, search_memory, …
│  (never pruned)              │  (always included regardless of score)
└──────────┬───────────────────┘
           │
           ▼
┌──────────────────────────────┐
│  Rerank candidates           │  cross-encoder scores each tool
│  against user prompt         │  against the latest user message
└──────────┬───────────────────┘
           │
           ▼
┌──────────────────────────────┐
│  Return top-N + protected    │  e.g. 15 tools instead of 90
└──────────────────────────────┘
```

## Configuration

All configuration is via environment variables.

| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_RERANKER_ENABLED` | `false` | Set `true` or `1` to enable |
| `CADE_RERANKER_TOP_N` | `15` | Maximum tools sent to the LLM |
| `CADE_RERANKER_BACKEND` | `local` | `local`, `cohere`, `voyage`, or `jina` |
| `CADE_RERANKER_MODEL_PATH` | *(auto)* | Override the local model directory |
| `COHERE_API_KEY` | — | Required when backend is `cohere` |
| `VOYAGE_API_KEY` | — | Required when backend is `voyage` |
| `JINA_API_KEY` | — | Required when backend is `jina` |

## Protected Tools

Certain tools are **never pruned** regardless of their reranking score.
These are the agent's lifeline for context recovery and core coding:

- `bash`, `read_file`, `ReadFileGemini`, `RunShellCommand`
- `search_memory`, `conversation_search`, `update_memory`, `update_memory_typed`
- `memory_apply_patch`, `archival_memory_insert`, `archival_memory_search`
- `ask_user_question`

## Backends

### Local (default)

Uses a local ONNX cross-encoder model (`ms-marco-MiniLM-L-6-v2`).

- **No API key needed**
- ~100ms latency on modern CPUs
- ~90 MB model downloaded on first use
- Runs entirely on-device

### Cloud Providers

| Backend | Model | Cost |
|---------|-------|------|
| `cohere` | `rerank-v3.5` | ~$0.10/1K queries |
| `voyage` | `rerank-2.5` | ~$0.05/1K queries |
| `jina` | `jina-reranker-v2-base-multilingual` | Free tier available |

Example:
```bash
export CADE_RERANKER_ENABLED=true
export CADE_RERANKER_BACKEND=cohere
export COHERE_API_KEY=your-key-here
```

## Token Savings

| Scenario | Tools | Tokens/Request | Savings |
|----------|-------|----------------|---------|
| Without ITS | 90 | ~13,500 | — |
| With ITS (top 15) | 15 | ~2,250 | **83%** |

For a session making 500 LLM requests/day with Claude Sonnet:
- Without ITS: ~$20/day in tool schema tokens
- With ITS: ~$3.40/day
- **Monthly savings: ~$500**

## Graceful Fallback

If the reranker encounters an error (model missing, API timeout, etc.),
it silently returns the **full, unfiltered tool set**.  The LLM call
proceeds normally — just without the token savings.

## Build Considerations

ITS is compiled into CADE when the `reranker` feature is active (part of
the default `full` feature set).  For minimal builds:

```bash
# Build without reranker to save ~8-12 MB binary size
cargo build --release --no-default-features --features "desktop,web,mcp,codeintel"
```

# ADR-0023: Gemini and Headroom Proxy Integration Strategy

## Status

Accepted

## Context

CADE integrates with [Headroom](https://github.com/headroom-ai/headroom) to provide token optimization, context compression, and transparent proxy caching:
1. **Native MCP Tools**: `headroom_compress`, `headroom_retrieve`, and `headroom_stats` operate model-agnostically by offloading large tool outputs, diffs, and compiler dumps out of the active context window into persistent storage.
2. **Transparent Network Proxy (`headroom proxy`)**: Listens on `http://127.0.0.1:8787` to transparently intercept, cache, and compress full-turn model prompts and responses on network round-trips.

Currently, Headroom's proxy intercepts standard OpenAI (`/v1/chat/completions`, `/v1/responses`) and Anthropic (`/v1/messages`) endpoints, as well as Google Vertex AI / Cloud Code internal publisher endpoints (`cloudcode-pa.googleapis.com` and `us-central1-aiplatform.googleapis.com`).

However, CADE's `GeminiProvider` (`crates/cade-ai/src/gemini.rs`) communicates with the Google AI Studio Developer API:
`https://generativelanguage.googleapis.com/v1beta/models/{model}:{action}?key={api_key}`

When users run CADE with Gemini models (e.g. `gemini/gemini-2.5-pro` or `gemini/gemini-3.8-flash`) via `cade-headroom`, requests currently bypass the local proxy daemon because the proxy does not route developer API endpoints.

## Decision

We adopt a **Dual-Track Hybrid Integration Architecture**:

1. **Model-Aware Transparent Proxy Routing**:
   - `AnthropicProvider` and `OpenAiProvider` dynamically route through `http://127.0.0.1:8787` via standard environment variables (`OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL`) or database `base_url` configuration without hardcoding.
   - `GeminiProvider` supports `GEMINI_BASE_URL` / `GOOGLE_AI_BASE_URL` overrides for enterprise proxies, but defaults to direct Google AI Studio routing for consumer developer keys.

2. **Model-Agnostic Context Compression via MCP**:
   - For Gemini sessions, context conservation and token reduction are handled via Headroom's native MCP tools (`headroom_compress` and `headroom_retrieve`).
   - Large tool outputs (file dumps, test outputs, search results) are collapsed into compact hash pointers, protecting Gemini's active context window without requiring network-level protocol translation.

3. **Explicit TUI Observability**:
   - The TUI status panel and sidebar display clear, non-ambiguous routing status:
     - When using OpenAI or Anthropic models with the proxy active: `proxy: headroom (8787)` (green).
     - When using Gemini models: `direct: google-ai` (dimmed).

4. **Future Upstream Headroom Route Expansion**:
   - Propose an upstream route handler in Headroom to intercept `generativelanguage.googleapis.com/v1beta` requests directly when Gemini-compatible tokenization and prompt caching primitives are added upstream.

## Consequences

### Positive
- **Zero Lossiness**: Gemini-specific features (native context caching `cachedContent`, `thought` parameter handling, search grounding) remain 100% reliable without being distorted by OpenAI translation bridges.
- **Total Transparency**: Users immediately understand why proxy compression metrics advance during OpenAI/Anthropic sessions and how Gemini sessions handle large payloads via MCP.
- **Architectural Purity**: Avoids introducing brittle local protocol translation layers inside CADE's core AI crate.

### Negative
- Transparent network-level prompt compression is not active for Gemini Developer API traffic until upstream Headroom provides native Google AI Studio route handlers.

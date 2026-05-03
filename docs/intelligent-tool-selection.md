# Intelligent Tool Selection (ITS)

CADE includes a built-in tool management system that reduces prompt token usage
on long conversations by pruning unused tools and compressing third-party tool
schemas.

## How It Works

ITS runs automatically inside `build_context` on every LLM request. No
configuration is needed.

```
All registered tool schemas (native + MCP)
    │
    ▼
┌──────────────────────────────────┐
│  Layer 1: desktop_* pruning      │  Remove desktop_* schemas if
│  (long sessions only)            │  unused in last 20 messages
└──────────┬───────────────────────┘
           │
           ▼
┌──────────────────────────────────┐
│  Layer 2: MCP schema compression │  Truncate descriptions of unused
│  (long sessions only)            │  MCP tools to 80 chars; strip
│                                  │  per-property descriptions
└──────────┬───────────────────────┘
           │
           ▼
  Final tool set sent to LLM
```

### Layer 1: Desktop Tool Pruning

Desktop tools (`desktop_screenshot`, `desktop_list_windows`, `desktop_control`,
`desktop_notify`) are removed entirely from the tool set when:

- The session has more than 20 messages, **and**
- None of these tools were called in the last 20 messages

This saves ~4 schema slots for sessions that never use desktop features.

### Layer 2: MCP Schema Compression

On long sessions (> 20 messages), MCP tool schemas (identified by the `__`
namespace separator in their name, e.g. `desktop-commander__read_file`) have
their descriptions compressed:

- Top-level `description` truncated to 80 characters (first line only)
- Per-property `description` fields removed
- Per-property `examples` fields removed
- `name`, parameter types, `required`, and enum values are preserved

**CADE-owned tools are never compressed.** All meta tools (memory, skills,
checkpoints, subagents) and native tools (bash, read_file, write_file, etc.)
always keep their full descriptions so the LLM can reliably understand and call
them.

## Token Savings

On a typical session with 7 MCP servers (~90 MCP tools):

| Scenario | Compressed tools | Tokens saved/request |
|----------|-----------------|---------------------|
| Short session (< 20 msgs) | 0 | 0 |
| Long session, few MCP calls | ~80 MCP tools | ~10,000–15,000 |
| Long session, many MCP calls | ~50 MCP tools | ~5,000–8,000 |

CADE-owned tools (~36 total) are never compressed, ensuring the agent always
has full context for its core capabilities.

## Constants

| Name | Value | Location |
|------|-------|----------|
| `RECENT_WINDOW` | 20 messages | `messages/mod.rs` |
| `COMPRESSED_DESCRIPTION_CHAR_CAP` | 80 chars | `messages/context.rs` |
| `EXTENDED_TOOL_PREFIXES` | `["desktop_"]` | `messages/context.rs` |

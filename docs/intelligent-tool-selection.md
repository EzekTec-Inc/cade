# Intelligent Tool Selection (ITS)

CADE includes a built-in tool management system that reduces prompt token usage
on long conversations by pruning unused tools and compressing third-party tool
schemas.

## How It Works

ITS runs automatically inside `build_context` on every LLM request. No
configuration is needed.

```
All registered tool schemas (with DB tags)
    │
    ▼
┌──────────────────────────────────┐
│  Layer 1: MCP desktop pruning    │  Remove MCP desktop_* schemas if
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

## Tool Classification

Tools are classified by their **DB registration tags** — not by hardcoded name
lists. The tag contract:

| Source | Tags | Compressed? | Pruned? |
|--------|------|-------------|---------|
| Meta tools (memory, skills, checkpoints, subagents) | `["cade", "meta"]` | Never | Never |
| Native tools (bash, read_file, write_file, etc.) | `["cade"]` | Never | Never |
| MCP tools (third-party, from MCP servers) | `["cade", "mcp"]` | Yes (if unused) | Yes (if desktop_* and unused) |

This means:
- **Adding a new meta tool** only requires adding its schema to `meta.rs` — ITS auto-discovers it.
- **Adding a new MCP server** requires no ITS changes — all MCP tools are auto-tagged `"mcp"` at registration.
- **No hardcoded tool name lists** exist in the ITS layer.

### Prefix-Agnostic Budget Limits

In addition to schema-level pruning and compression, CADE automatically and dynamically regulates token budgets for tool execution outputs. In `tool_output_limit` (`messages/mod.rs`), CADE strips any MCP server-prefixed namespaces dynamically on the first `__` separator, mapping them to standard action categories (e.g., `bash`, `read_file`, `grep`, `glob`, `git`) to apply precise size-caps:
- **Stdio & Shell Execution** (`bash`): Capped at 4,096 characters.
- **File Reading** (`read_file`): Expanded to 12,288 characters to allow parsing full context.
- **Search & Grep** (`grep`): Capped at 3,072 characters to output compact matches.
- **Directories & Globs** (`glob`): Capped at 3,072 characters.
- **Everything Else**: Defaults to the standard tool results limit (8,192 characters).

No hardcoded prefix lists or server names are used in the budget regulation layer.

### Layer 1: Desktop Tool Pruning

MCP tools with a `desktop_*` name prefix are removed entirely from the tool set
when:

- The session has more than 20 messages, **and**
- None of these tools were called in the last 20 messages

This saves schema slots for sessions that never use desktop MCP features.

### Layer 2: MCP Schema Compression

On long sessions (> 20 messages), MCP tool schemas (tagged `"mcp"`) have their
descriptions compressed when not recently used:

- Top-level `description` truncated to 80 characters (first line only)
- Per-property `description` fields removed
- Per-property `examples` fields removed
- `name`, parameter types, `required`, and enum values are preserved

**CADE-owned tools are never compressed.** All meta tools and native tools
always keep their full descriptions so the LLM can reliably understand and call
them.

## Sequential Tool Classification

In headless mode, meta tools are classified as sequential (cannot run in
parallel) to prevent race conditions on shared agent state. This classification
is discovered dynamically from the `all_meta_schemas()` registry via a
`LazyLock` set — no hardcoded tool name list.

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

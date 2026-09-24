# ADR-0026: OpenAI GPT-6 Dual-Wire Routing, Deep MCP Normalization, and Interactive Permission Seams

## Status

Accepted

## Context

Several architectural and runtime gaps emerged as CADE transitioned to a decoupled client/server model and integrated next-generation frontier reasoning models:

1. **OpenAI GPT-6 Chat Completions 400 Bad Request**:
   When invoking `openai/gpt-6-*` (e.g. `openai/gpt-6-sol`) with function tools and active `reasoning_effort`, the upstream OpenAI API rejected calls with `HTTP 400 Bad Request: Function tools with reasoning_effort are not supported for gpt-6-sol in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'`. CADE's legacy model classifier only routed `gpt-5*` models to `/v1/responses`.
2. **Permission System Client/Server Split-Brain**:
   While the terminal CLI loaded user rules (`allow: ["Bash(cargo test)"]`, `deny: ["Bash(rm:*)"]`) from `~/.cade/settings.json`, tool execution was owned by `cade-server`. The server constructed a hardcoded blank `PermissionManager` and attached `AutoApprovalDelegate`, which unconditionally returned `Ok(true)` and bypassed all confirmation prompts.
3. **`ask_user_question` Disconnect**:
   The `AskUserQuestionTool` was historically handled via local REPL interception. Moving execution to `cade-server` left it with no transport to pause the turn loop and present the question modal to the client.
4. **Daemon MCP Path Resolution Failures**:
   Background MCP server processes (such as `cade-rag-mcp` and `serena`) run with independent daemon working directories. Relative arguments passed by LLMs (e.g. `"path": "."`) failed with `Index not found` because paths were not canonicalized against the caller's active workspace.

## Decision

We establish four deep, unified architectural seams:

### 1. Unified `OpenAiModelCapabilities` Classifier
- Replace scattered prefix checks with `OpenAiModelCapabilities::for_model`.
- When `gpt-6*` or `gpt-5*` models are invoked with function tools:
  - Route through the `/v1/responses` wire protocol.
  - Transform chat messages into the Responses `input` array.
  - Serialize function tools as flat Responses tool definitions.
  - Map reasoning configuration to `reasoning: { effort: "<effort>" }` and tokens to `max_output_tokens`.
- Preserve verified `/v1/chat/completions` behavior for `o1*`, `o3*`, `o4*` (top-level `reasoning_effort` and `max_completion_tokens`) and legacy models (`gpt-4o` with `max_tokens`).

### 2. Authoritative Workspace Permission Loading on Server
- Initialize server execution `PermissionManager` directly from the active workspace `SettingsManager`.
- Populate canonical `allow` and `deny` rules on every turn.
- Transmit session-level `permission_mode` overrides (`Plan`, `AcceptEdits`, `Default`, `BypassPermissions`) over `POST /v1/agents/:id/run`.
- Automatically reject denied tools and block mutating writes in Plan mode before tool invocation.

### 3. Asynchronous `SseApprovalDelegate` & `InteractionDelegate`
- Replace `AutoApprovalDelegate` with `SseApprovalDelegate`:
  - On `Verdict::Ask`, create a pending approval in SQLite and emit `approval_required` over the active SSE stream.
  - Asynchronously yield on database status with exponential backoff until approved, denied, or timed out.
  - Re-use `POST /v1/approvals/:id/action` to resume execution without new endpoints.
- Wire `InteractionDelegate` into `AskUserQuestionTool`:
  - Emits `question_required` over SSE.
  - Displays interactive prompt or modal in the client TUI/GUI.
  - Resumes execution with formatted user answers via approval feedback payload (`approved:<answers>`).

### 4. Deep `McpArgumentNormalizer` & `McpContentExtractor`
- `McpArgumentNormalizer`: Automatically expands `.`, `./`, `~`, and relative paths against the active workspace directory for all MCP tools.
- `McpContentExtractor`: Standardizes multi-format MCP outputs (`RawContent::Text`, embedded `TextResourceContents`, `BlobResourceContents`, `ResourceLink`) and automatically strips debug sampling fallback headers.

## Consequences

- **Correctness**: Eliminates the OpenAI 400 Bad Request error on GPT-6 models while maintaining 100% regression safety across all other model families.
- **Safety**: Restores human-in-the-loop authorization, deny rules, and Plan mode sandboxing across the client/server seam.
- **Interactivity**: Restores clarifying questions (`ask_user_question`) directly in the streaming terminal without breaking headless test automation.
- **Reliability**: Guarantees consistent filesystem resolution across all daemon and local MCP servers.
- **Clean Architecture**: Follows Codebase Design and Rust10x zero-panic standards across all 15 crates.

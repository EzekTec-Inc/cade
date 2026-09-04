# CADE Rust SDK API Reference

Exhaustive API reference and type documentation for the `cade-sdk` crate.

---

## 1. Top-Level Exports

```rust
use cade_sdk::{
    // In-Process Zero-Daemon Execution
    EmbeddedSession,
    EmbeddedSessionBuilder,

    // Multi-Agent Squad Orchestration
    TeamSession,
    TeamSessionBuilder,

    // Daemon Client-Server Connections
    AgentSession,
    SessionOptions,
    CadeClientSdk,

    // Typed Reactive Events & Errors
    CadeStreamEvent,
    Error,
    Result,
};
```

---

## 2. In-Process Runtime: `EmbeddedSession`

### `EmbeddedSessionBuilder`

Builder pattern for configuring an in-process agent runtime.

| Method | Parameters | Description |
|---|---|---|
| `new()` | None | Creates a new builder with default settings (`claude-sonnet-4-5`, in-memory DB). |
| `model(...)` | `impl Into<String>` | Sets the model identifier (e.g. `"anthropic/claude-sonnet-4-5"`, `"openai/gpt-4o"`, `"ollama/deepseek-r1"`). |
| `system_prompt(...)` | `impl Into<String>` | Sets custom system instructions for the agent. |
| `db_path(...)` | `impl Into<PathBuf>` | Path to a persistent SQLite database file (defaults to `:memory:`). |
| `in_memory()` | None | Explicitly forces in-memory SQLite storage. |
| `agent_id(...)` | `impl Into<String>` | Explicit agent ID string (defaults to auto-generated UUID). |
| `agent_name(...)` | `impl Into<String>` | Display name of the agent. |
| `cwd(...)` | `impl Into<PathBuf>` | Working directory for tool and skill path resolution. |
| `permission_mode(...)` | `PermissionMode` | Permission mode (`BypassPermissions`, `Default`, `PlanOnly`). |
| `allowed_paths(...)` | `Vec<String>` | Granular path whitelist for filesystem sandboxing. |
| `ai_config(...)` | `AiConfig` | Custom AI API keys and base URL configurations. |
| `max_turns(...)` | `usize` | Maximum tool execution turns allowed per prompt (default: 20). |
| `build().await` | None | Builds and initializes the `EmbeddedSession`. |

### `EmbeddedSession`

| Method | Signature | Description |
|---|---|---|
| `prompt` | `async fn prompt(&mut self, text: &str) -> Result<String>` | Dispatches a prompt, executes necessary tool calls, and returns the final text response. |
| `prompt_stream` | `async fn prompt_stream(&mut self, text: &str) -> Result<Pin<Box<dyn Stream<Item = CadeStreamEvent> + Send>>>` | Dispatches a prompt and returns a stream of real-time `CadeStreamEvent`s. |
| `prompt_with_history` | `async fn prompt_with_history(&mut self, text: &str, conversation_id: Option<&str>) -> Result<String>` | Runs a prompt within a specific conversation thread. |
| `get_memory` | `async fn get_memory(&self) -> Result<Vec<MemoryBlock>>` | Retrieves all persistent memory blocks for the active agent. |
| `search_memory` | `async fn search_memory(&self, query: &str, memory_type: Option<&str>) -> Result<Vec<serde_json::Value>>` | Performs hybrid vector + BM25 search across agent memory. |
| `agent_id` | `fn agent_id(&self) -> &str` | Returns the agent's unique ID. |
| `model` | `fn model(&self) -> &str` | Returns the active model name. |
| `db` | `fn db(&self) -> &Db` | Accesses the underlying SQLite connection handle. |

---

## 3. Multi-Agent Squads: `TeamSession`

### `TeamSessionBuilder`

| Method | Parameters | Description |
|---|---|---|
| `new()` | None | Creates a new team builder. |
| `model(...)` | `impl Into<String>` | Sets the primary model for subagents. |
| `build().await` | None | Initializes the `TeamSession`. |

### `TeamSession`

| Method | Signature | Description |
|---|---|---|
| `run_team` | `async fn run_team(&self, prompt: &str) -> Result<String>` | Decomposes the task across a supervisor and worker tree, returning aggregated output. |
| `run_team_stream` | `async fn run_team_stream(&self, prompt: &str) -> Result<Pin<Box<dyn Stream<Item = CadeStreamEvent> + Send>>>` | Streams multi-agent activity, tool invocations, and deltas live. |

---

## 4. Daemon Client: `CadeClientSdk`

Direct cross-platform HTTP client for communicating with a running `cade-server`.

| Method | Signature | Description |
|---|---|---|
| `new` | `fn new(server_url: String, api_key: String) -> Self` | Instantiates a client pointed at a CADE server. |
| `list_agents` | `async fn list_agents(&self) -> Result<Vec<AgentInfo>>` | Lists all active agents on the server. |
| `get_messages` | `async fn get_messages(&self, agent_id: &str, conv_id: Option<&str>) -> Result<Vec<ChatMessage>>` | Fetches historical messages for an agent. |
| `stream_messages` | `async fn stream_messages(&self, agent_id: &str, input: &str, conv_id: Option<&str>) -> Result<BoxStream<'static, Result<StreamEvent>>>` | Connects to the SSE stream endpoint. |
| `list_conversations` | `async fn list_conversations(&self, agent_id: &str) -> Result<Vec<ConversationInfo>>` | Lists conversation threads. |
| `create_conversation` | `async fn create_conversation(&self, agent_id: &str, title: Option<&str>) -> Result<ConversationInfo>` | Creates a new conversation thread. |
| `get_memory` | `async fn get_memory(&self, agent_id: &str) -> Result<Vec<serde_json::Value>>` | Fetches memory blocks. |
| `save_memory` | `async fn save_memory(&self, agent_id: &str, label: &str, value: &str) -> Result<()>` | Persists a memory block. |
| `delete_memory` | `async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()>` | Deletes a memory block. |

---

## 5. Reactive Events: `CadeStreamEvent`

Strongly-typed stream events emitted during agent turns:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CadeStreamEvent {
    /// Incremental reasoning/thinking delta from the model.
    Thought(String),
    /// Incremental assistant text chunk.
    MessageDelta(String),
    /// Tool invocation started.
    ToolExecuting {
        tool_call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// Tool execution completed.
    ToolCompleted {
        tool_call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
    /// Human-in-the-loop approval requested.
    ApprovalRequired {
        approval_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// Cumulative token usage metrics.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        model: String,
    },
    /// Turn completed with final reason.
    Finished { outcome: String },
    /// Execution error.
    Error(String),
}
```

---

## 6. Error Handling

All SDK operations return `Result<T, cade_sdk::Error>`.

```rust
match session.prompt("Run analysis").await {
    Ok(output) => println!("{output}"),
    Err(cade_sdk::Error::Custom(msg)) => eprintln!("SDK Error: {msg}"),
    Err(e) => eprintln!("Error: {e}"),
}
```

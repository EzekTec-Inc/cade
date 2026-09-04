# CADE SDK Quickstart Guide

This guide walks you through building your first autonomous agent applications with `cade-sdk`.

---

## 1. Prerequisites & Environment Setup

Ensure you have a Rust toolchain installed (edition 2024 / 2021 compatible, Rust $\ge 1.85$).

Export your preferred LLM provider API key in your terminal environment:

```bash
# Pick any provider
export ANTHROPIC_API_KEY="sk-ant-..."
# or
export OPENAI_API_KEY="sk-..."
# or
export GEMINI_API_KEY="AIzaSy..."
# or point to a local Ollama instance (default: http://localhost:11434)
```

---

## 2. Setting Up Your Cargo Project

Create a new binary application:

```bash
cargo new my-cade-agent
cd my-cade-agent
```

Add dependencies to `Cargo.toml`:

```toml
[dependencies]
cade-sdk = { path = "/path/to/CADE/crates/cade-sdk" }
tokio = { version = "1", features = ["full"] }
futures = "0.3"
serde_json = "1"
```

---

## 3. Pattern A: In-Process Zero-Daemon Agent (`EmbeddedSession`)

The `EmbeddedSession` runs the entire CADE agentic loop, tool runtime, and local SQLite memory store directly inside your process without requiring a running `cade-server` daemon.

### Basic In-Memory Agent

Create `src/main.rs`:

```rust
use cade_sdk::EmbeddedSession;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build an embedded session (in-memory SQLite by default)
    let mut session = EmbeddedSession::builder()
        .model("anthropic/claude-sonnet-4-5")
        .system_prompt("You are an expert Rust software architect.")
        .build()
        .await?;

    // 2. Dispatch a prompt and await completion
    let response = session
        .prompt("Inspect the current workspace and suggest three optimizations.")
        .await?;

    println!("Agent Response:\n{response}");
    Ok(())
}
```

### Persistent Embedded Agent with Sandboxing

To persist memory across runs and restrict file system access to a specific project directory:

```rust
use cade_sdk::EmbeddedSession;
use cade_core::permissions::PermissionMode;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = EmbeddedSession::builder()
        .db_path("./agent-state.db")                         // Persistent SQLite database
        .agent_id("lead-architect-01")                     // Stable agent identity
        .cwd(PathBuf::from("/path/to/my/project"))         // Project working directory
        .allowed_paths(vec!["/path/to/my/project/src".into()]) // Granular sandboxing
        .permission_mode(PermissionMode::Default)
        .build()
        .await?;

    println!("Persistent agent initialized: {}", session.agent_id());
    Ok(())
}
```

---

## 4. Pattern B: Real-Time Event Streaming

To stream thinking deltas, tool executions, and tokens live (e.g., for terminal TUIs or web UIs):

```rust
use cade_sdk::{CadeStreamEvent, EmbeddedSession};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = EmbeddedSession::builder()
        .model("anthropic/claude-sonnet-4-5")
        .build()
        .await?;

    // Stream typed events
    let mut stream = session
        .prompt_stream("Refactor src/utils.rs to use iterator chains.")
        .await?;

    while let Some(event) = stream.next().await {
        match event {
            CadeStreamEvent::Thought(reasoning) => {
                print!("\x1b[35m[Thinking]\x1b[0m {reasoning}");
            }
            CadeStreamEvent::MessageDelta(chunk) => {
                print!("{chunk}");
            }
            CadeStreamEvent::ToolExecuting { tool_name, arguments, .. } => {
                println!("\n\x1b[36m⚡ Tool Invoked:\x1b[0m {tool_name}({arguments})");
            }
            CadeStreamEvent::ToolCompleted { tool_name, is_error, output, .. } => {
                let status = if is_error { "\x1b[31mFailed\x1b[0m" } else { "\x1b[32mOK\x1b[0m" };
                println!("↳ Result [{status}]: {} chars", output.len());
            }
            CadeStreamEvent::Finished { outcome } => {
                println!("\n\x1b[32m✔ Completed:\x1b[0m {outcome}");
            }
            CadeStreamEvent::Error(err) => {
                eprintln!("\n\x1b[31m✘ Error:\x1b[0m {err}");
            }
            _ => {}
        }
    }

    Ok(())
}
```

---

## 5. Pattern C: Multi-Agent Squads (`TeamSession`)

To orchestrate a collaborative team of specialized subagents (e.g. Researcher + Coder + Reviewer):

```rust
use cade_sdk::TeamSession;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = TeamSession::builder()
        .model("anthropic/claude-sonnet-4-5")
        .build()
        .await?;

    let result = session
        .run_team("Audit the authentication flow, identify vulnerabilities, and generate patch tests.")
        .await?;

    println!("Team Orchestration Summary:\n{result}");
    Ok(())
}
```

---

## 6. Pattern D: Daemon-Connected Client (`CadeClientSdk`)

If you have a centralized `cade-server` running on `http://localhost:8284`:

```rust
use cade_sdk::CadeClientSdk;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = CadeClientSdk::new(
        "http://localhost:8284".to_string(),
        "my-secret-api-token".to_string(),
    );

    // List agents on remote server
    let agents = client.list_agents().await?;
    println!("Found {} active agents on server.", agents.len());

    // Stream messages over SSE
    let mut stream = client
        .stream_messages("default", "Hello from remote SDK!", None)
        .await?;

    while let Some(item) = stream.next().await {
        if let Ok(event) = item {
            if let Some(text) = event.content() {
                print!("{text}");
            }
        }
    }

    Ok(())
}
```

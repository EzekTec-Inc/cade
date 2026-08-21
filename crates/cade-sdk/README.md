# CADE SDK (`cade-sdk`)

Rust SDK for embedding CADE in standalone applications, CLI tools, serverless functions (AWS Lambda, Cloud Run), and multi-agent systems with zero external daemon requirements.

## Key Capabilities

1. **Embedded In-Process Execution (`EmbeddedSession`)**:
   - Zero-daemon runtime linking directly to `cade-store` SQLite (`:memory:` or on-disk) and `cade-ai` (`LlmRouter`).
   - Executes multi-turn tool loops to convergence without network ports or HTTP latency.
   - Built-in support for memory persistence, skill loading, and sandboxed file I/O.

2. **Declarative Multi-Agent Squads (`TeamSession`)**:
   - Programmatically configure collaborative squads (Coordinator, Lead Architect, Coder, Reviewer, Security Oracle).
   - Enforce tool access policies, custom system prompts, and squad execution modes (`Coordinate`, `Route`, `Tasks`).

3. **Strongly-Typed Stream Telemetry (`CadeStreamEvent`)**:
   - Stream type-safe events (`Thought`, `MessageDelta`, `ToolExecuting`, `ToolCompleted`, `ApprovalRequired`, `Usage`, `Finished`, `Error`).
   - Eliminate raw JSON regexes and build reactive UI progress bars and diffs directly.

4. **Remote Daemon Control (`AgentSession`)**:
   - Connect over HTTP/SSE to external `cade-server` instances for centralized deployments.

---

## Quickstart

### 1. In-Process Embedded Agent (`EmbeddedSession`)

```rust
use cade_sdk::{EmbeddedSession, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Build an in-process session backed by in-memory SQLite
    let session = EmbeddedSession::builder()
        .in_memory()
        .model("anthropic/claude-sonnet-4-5")
        .system_prompt("You are an expert Rust systems architect.")
        .build()
        .await?;

    // 2. Execute a multi-turn prompt loop to convergence
    let answer = session.prompt("Inspect Cargo.toml and list dependencies").await?;
    println!("Assistant: {answer}");

    // 3. Persistent memory management
    session.set_memory("project_rules", "Strict TDD and 100% test coverage").await?;
    let memory_val = session.get_memory("project_rules").await?;
    println!("Memory: {:?}", memory_val);

    Ok(())
}
```

### 2. Strongly-Typed Event Streaming

```rust
use cade_sdk::{CadeStreamEvent, EmbeddedSession, Result};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    let session = EmbeddedSession::builder()
        .in_memory()
        .model("anthropic/claude-sonnet-4-5")
        .build()
        .await?;

    let mut stream = session.stream_prompt("Analyze workspace architecture").await?;

    while let Some(event) = stream.next().await {
        match event {
            CadeStreamEvent::Thought(thought) => {
                println!("[Thinking]: {thought}");
            }
            CadeStreamEvent::MessageDelta(chunk) => {
                print!("{chunk}");
            }
            CadeStreamEvent::ToolExecuting { tool_name, .. } => {
                println!("\n[Tool Starting]: {tool_name}");
            }
            CadeStreamEvent::ToolCompleted { tool_name, is_error, .. } => {
                println!("[Tool Finished]: {tool_name} (error: {is_error})");
            }
            CadeStreamEvent::Usage { input_tokens, output_tokens, model } => {
                println!("\n[Usage]: {input_tokens} in, {output_tokens} out ({model})");
            }
            CadeStreamEvent::Finished { outcome } => {
                println!("\n[Finished]: {outcome}");
            }
            CadeStreamEvent::Error(err) => {
                eprintln!("\n[Error]: {err}");
            }
            _ => {}
        }
    }

    Ok(())
}
```

### 3. Multi-Agent Team Squad (`TeamSession`)

```rust
use cade_sdk::{TeamSession, Result};
use cade_agent::team::{MemberTools, TeamMode};

#[tokio::main]
async fn main() -> Result<()> {
    let squad = TeamSession::builder()
        .team_id("security-review-team")
        .name("Security Audit Squad")
        .mode(TeamMode::Coordinate)
        .with_member(
            "architect",
            "Lead Architect",
            "System architecture and threat modeling",
            "You are an expert system architect.",
            MemberTools::Readonly,
        )
        .with_member(
            "security_oracle",
            "Security Oracle",
            "Vulnerability discovery and automated audit",
            "You are a strict security auditor.",
            MemberTools::Readonly,
        )
        .build()
        .await?;

    let results = squad.run("Audit authentication flow for timing attacks").await?;

    for item in results {
        println!("Task {} Output: {}", item.task_index, item.output);
    }

    Ok(())
}
```

---

## License

Dual-licensed under MIT or Apache-2.0.

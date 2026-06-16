# CADE LLM & Provider Architecture: Custom vs. genai-rs vs. Rig

This document provides an exhaustive, production-grade architectural review comparing CADE's native provider engine (`crates/cade-ai` and `crates/cade-agent`) with two prominent open-source Rust AI ecosystems:
1. **`genai-rs`** ([github.com/evansenter/genai-rs](https://github.com/evansenter/genai-rs)): A unified, stateless low-level chat completion client.
2. **`Rig`** ([github.com/0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig)): An all-in-one, highly modular, agentic and RAG application framework.

---

## 1. Executive Summary

CADE's current LLM integration represents a **bespoke, stateful agentic system** built specifically for full-scale terminal and GUI development. To evaluate its long-term scalability, we compare CADE against the two prevailing paradigms in the Rust LLM space:

* **`genai`** represents the **Stateless Completion Paradigm**. It standardizes API calls to multiple model providers but remains unopinionated about agent states, memory, or databases.
* **`Rig`** represents the **Stateful Agent & RAG Paradigm**. It unifies LLM providers, vector databases, tool-calling pipelines, and agent preambles into a cohesive, high-level declarative framework.

### Three-Way Architectural Matrix

| Dimension | CADE Custom Implementation | `genai-rs` Library | `Rig` Agentic Framework |
| :--- | :--- | :--- | :--- |
| **Abstaction Tier** | **Mid-to-High Hybrid**<br>Decouples provider traits (`cade-ai`) from agent loops and turn compaction (`cade-agent`). | **Low-Level Completion Client**<br>Standardizes basic completions, streaming, and embeddings across APIs. | **High-Level Agent Framework**<br>Declarative builder APIs for stateful `Agent`, `VectorStore`, and RAG pipelines. |
| **Design Philosophy** | Built specifically for dynamic agentic loops, MCP-driven tool dispatch, and state recovery. | Centralized HTTP engine calling stateless, translation-only `Adapter` traits. | Modular, compile-time typed primitives designed for modular AI architectures. |
| **Tool Calling / Functions** | **Dynamic & Runtime-Driven**<br>MCP schemas loaded over stdio/SSE are dispatched dynamically in the server event loop. | **Absent (Low-level focus)**<br>Requires the caller to manually parse tool calls and formats returned by the model. | **Static & Compile-Time Typed**<br>Tools defined via Rust traits and macros, validated at compile-time using `schemars`. |
| **Vector DB & RAG** | **Tightly Coupled**<br>Deeply integrates local SQLite + `fastembed`/`sqlite-vec` directly for memory persistence. | **Supported (Low-level)**<br>Provides basic embedding endpoints but does not connect to vector indices. | **Decoupled & Comprehensive**<br>10+ native vector store integrations (Qdrant, LanceDB, PGVector, Neo4j, etc.). |
| **Hot-Reloading** | **Native**<br>Mutable, thread-safe `LlmRouter` updates keys and providers at runtime via CLI/API. | **Limited**<br>Primarily designed for static initialization on startup via immutable client builders. | **Limited**<br>Configured programmatically. Adding/removing models on-the-fly is not its primary idiom. |
| **Maintenance Cost** | **High**<br>Manual updates needed for every upstream API schema change (e.g. GPT-5 Responses API). | **Low**<br>Boilerplate and model catalog additions are offloaded to library maintainers. | **Medium**<br>We rely on active open-source support to track upstream API changes across providers and DBs. |

---

## 2. Core Abstractions & Architectural Philosophies

### 2.1 CADE: The Stateful Agentic Workspace
CADE divides its AI stack into two clear, cooperative layers:
1. **`crates/cade-ai` (Low-to-Mid):** Implements the `LlmProvider` trait, which defines raw `complete` and `stream` async structures.
2. **`crates/cade-agent` (High):** Orchestrates the full stateful agent lifecycle, including system prompt packing, context window compaction, automatic fact extraction, and SQLite conversation persistence.

This architecture treats the LLM as an active, stateless compute engine, while CADE serves as the compiler/runtime that preserves state across context limits.

### 2.2 `genai-rs`: The Stateless Connection Pool
`genai` focuses exclusively on reducing HTTP boilerplate. It implements the stateless **Adapter Pattern**:
* The core `Client` owns the unified `reqwest::Client` connection pool and manages request middleware.
* The `Adapter` trait exposes transformation functions that compile standard library requests (e.g., `ChatRequest`) into provider-specific HTTP bodies, and map JSON responses back.

```rust
// genai-rs stateless transformation
let web_request_data = adapter::to_web_request_data(model, service, req, options)?;
let raw_response = self.web_client.execute(web_request_data).await?;
let chat_response = adapter::to_chat_response(raw_response)?;
```

This is a brilliant approach for general-purpose chat clients, but it places the entire burden of tool execution, memory, state management, and orchestration on CADE.

### 2.3 `Rig`: The High-Level Declarative Pipeline
`Rig` provides a complete, production-grade agentic framework that unifies models, tools, and embeddings under a declarative, builder-centric interface:

```rust
// Declarative Agent Builder in Rig
let agent = client
    .agent(openai::GPT_4O)
    .preamble("You are a specialized engineering assistant.")
    .tool(MyCustomTool)
    .build();

let response = agent.prompt("Analyze this codebase.").await?;
```

Rig abstracts providers and databases, allowing developers to swap both underlying LLM engines and vector databases with minimal code changes.

---

## 3. Deep Dive: Tool Calling & Dynamic Extensibility

A major architectural divergence between Rig and CADE lies in how tools (functions) are defined, verified, and executed.

### 3.1 Rig's Compile-Time Type Safety
Rig leverages Rust's strong type system and macros to define tools at compile-time:

```rust
// Rig tool definition pattern
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct AddArgs {
    x: i32,
    y: i32,
}

#[rig::tool]
fn add(args: AddArgs) -> i32 {
    args.x + args.y
}
```

* **Advantages:** Unmatched type-safety, automatic JSON schema generation via `schemars`, and zero runtime serialization errors.
* **Limitations:** Highly static. Tools must be compiled directly into the binary. Adding tools dynamically at runtime is extremely difficult.

### 3.2 CADE's Runtime MCP-Driven Orchestration
Because CADE is built to work with **Model Context Protocol (MCP)** servers, its tool registry is inherently **dynamic**:
* Tools are queried at runtime from external processes over stdio or SSE.
* Tool definitions are stored and mapped dynamically as raw JSON values (`serde_json::Value`).
* CADE's server event loop intercepts, validates, and dispatches these tool executions dynamically.

Therefore, Rig's static, compile-time tool-definition pattern is **incompatible with CADE's runtime-driven MCP tool architecture**.

### 3.3 OpenAI Responses API and OpenRouter Runtime Notes

CADE keeps bespoke provider serialization logic because upstream OpenAI-compatible
APIs do not all accept the same tool schema. For GPT-5-style OpenAI Responses
API requests, function tools are serialized in the flat Responses API shape:

```json
{
  "type": "function",
  "name": "tool_name",
  "description": "Tool description",
  "parameters": {},
  "strict": false
}
```

Keeping `name`, `description`, and `parameters` at the tool-object top level is
required for Responses API compatibility; nesting them under a secondary
`function` object can produce upstream validation failures such as missing
`tools[0].name`. CADE sets `strict` to `false` deliberately because many tools
are discovered from MCP servers at runtime and may include optional fields or
loose nested schemas that should not be rejected by strict OpenAI validation.

OpenAI-compatible providers may also enforce request-specific tool limits. CADE
caps OpenAI tool payloads at 128 tools and applies priority filtering before
truncation so essential meta-tools and MCP tools are preserved. This prevents
critical capabilities such as `load_skill` and server-prefixed MCP tools from
being dropped when a large workspace exposes more tools than OpenAI accepts.

OpenRouter is implemented as an OpenAI-compatible provider with provider-prefix
routing. Models addressed as `openrouter/...` are resolved to the OpenRouter
provider, have the provider prefix stripped before the upstream request, and use
OpenRouter's model catalogue response shape (`data: [{ id: ... }]`) for dynamic
model discovery.

---

## 4. Deep Dive: Vector Databases and Retrieval-Augmented Generation (RAG)

### 4.1 CADE's Integrated SQLite Engine
CADE implements an optimized, self-contained storage layer (`crates/cade-store`) built specifically for local workspace state:
* Direct integration of `fastembed` for local embedding generation.
* Uses SQLite with the `sqlite-vec` extension for FTS5 (Full-Text Search) and vector search.
* **Tight Coupling:** The database manages conversational history, credentials encryption, and active memories, keeping the entire runtime in a single, local `.db` file.

### 4.2 Rig's Decoupled Vector Connectors
Rig provides a plug-and-play vector store abstraction:
* Exposes the `VectorStoreIndex` trait.
* Native connectors for over 10 vector databases (LanceDB, Qdrant, PGVector, Chroma, MongoDB, etc.).
* Swapping vector backends is as simple as switching features in `Cargo.toml`.

For enterprise deployments where memory needs to scale to distributed indices (like Qdrant or MongoDB), Rig's decoupled abstraction is significantly superior to CADE's tight SQLite coupling.

---

## 5. Strategic Synthesis & Recommendations

### 5.1 CADE vs. genai-rs vs. Rig

| Feature | CADE Custom | `genai-rs` | `Rig` |
| :--- | :--- | :--- | :--- |
| **Completions** | Custom, tailored to agentic state | Comprehensive, stateless | Comprehensive, integrated |
| **Embeddings** | Handled in `cade-store` | Supported (raw endpoints) | Supported, integrated with Index |
| **RAG / Vector Stores** | Tied to SQLite | None | Modular, multi-DB support |
| **Agentic Loop** | Custom (compacting, stateful) | None | Standard (declarative preambles) |
| **Tool Execution** | Dynamic (MCP compat) | None | Static (compile-time Rust) |

---

## 6. Evolutionary Path: Recommendation for CADE

### Recommendation: Do Not Adopt `genai-rs` or `Rig` as Core Dependencies; instead, adopt their Architectural Patterns

**Justification:**
1. **The MCP Disconnect:** `Rig`'s compile-time tool-definition paradigm directly conflicts with CADE's runtime-driven, dynamic MCP server integration.
2. **Proprietary Agent State Guardrails:** Standardizing on a general-purpose library like `genai` or `Rig` would block CADE from implementing highly customized reasoning handlers, such as **Gemini's thought signatures** and **GPT-5's Responses API**, which require bespoke serialization control.
3. **Local Self-Containment:** CADE's unique value proposition is its highly localized, zero-dependency SQLite-backed workspace persistence, making Rig's extensive distributed database connectors overkill for the primary CLI/TUI use-case.

### How CADE Should Evolve (The Hybrid Refactoring Strategy)

While CADE should remain independent of these libraries to maintain its specialized agentic control, it should aggressively refactor its internal code to implement their **best architectural features**:

#### 1. Decouple HTTP Orchestration from Serializers (Adopt `genai`'s Stateless Adapters)
Refactor `crates/cade-ai` to separate the shared web request engine from individual provider formatting. 
Create an internal `Adapter` trait in CADE to define pure, stateless payload maps, reducing duplicated client-building and retry code.

#### 2. Decouple the Vector Store Interface (Adopt `Rig`'s Vector Abstractions)
Refactor `crates/cade-store` to abstract the embedding generator and vector database behind a `VectorIndex` trait:

```rust
#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn insert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value) -> Result<()>;
    async fn search(&self, query_vector: Vec<f32>, limit: usize) -> Result<Vec<SearchResult>>;
}
```

Implement the current local SQLite+`sqlite-vec` engine as the default implementation of this trait. This keeps CADE's local self-containment intact while opening the door for enterprise adapters (like Qdrant or PGVector) in the future.

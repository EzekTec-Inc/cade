# CADE LLM Provider Architecture vs. genai-rs

This document provides a detailed architectural review and comparative analysis of CADE's current LLM & provider implementation compared to the **genai-rs** library ([github.com/evansenter/genai-rs](https://github.com/evansenter/genai-rs)), authored by Jeremy Chone (`genai` on crates.io).

---

## 1. Executive Summary

CADE's LLM engine (`crates/cade-ai`) is currently built around a custom `LlmProvider` trait with self-contained, independent implementation files for each major provider (`openai`, `anthropic`, `gemini`, `ollama`).

`genai` is an increasingly popular, production-grade Rust client library that unifies generative AI interactions through a decoupled, stateless **Adapter** pattern where HTTP orchestration is centralized in a single `Client` and providers implement pure request/response data mappings.

### High-Level Comparison Matrix

| Architectural Dimension | CADE Custom Implementation | `genai-rs` Library |
| :--- | :--- | :--- |
| **Code Splitting & Design** | Isolated monolithic providers. Each provider manages its own HTTP clients, auth, serialization, and stream processing. | Highly decoupled and stateless. Core `Client` handles HTTP lifecycle; `Adapter` trait manages data transformations. |
| **API Code Duplication** | High. Significant duplicate boilerplate for client building, headers, backoff retry logic, and JSON parsing. | Zero. Standard HTTP orchestration is written once in the core client. Adapters are thin translation layers. |
| **Agentic Core Features** | First-class support for CADE-specific multi-part payloads, precise token caching, Gemini thinking signatures, and GPT-5 Responses API. | General-purpose chat focus. Embedding support included out-of-the-box, but advanced multi-modal or proprietary features require customized wrappers. |
| **Hot-Reloading Capabilities** | Built natively for runtime hot-swaps via thread-safe `HashMap` in `LlmRouter`, supporting concurrent live API model listings. | Primarily designed for static Client building. Runtime adjustments require configuring custom runtime mapping functions. |
| **Maintenance Overhead** | High. CADE must manually update request/response types for every minor upstream provider API change. | Low. Upstream API updates and new provider adapters are offloaded to library maintainers. |

---

## 2. Core Interface & Trait System Deep Dive

### 2.1 CADE's Custom `LlmProvider`

CADE utilizes a single, high-level asynchronous trait representing a stateful, fully-capable client provider:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;
}
```

* **Linear Flow:** Both standard completions and streaming completions are fully-integrated async methods. Calling `stream` yields a pinned box of `StreamChunk` items.
* **Encapsulation:** The implementing struct (e.g., `OpenAiProvider`) owns its API keys, customized endpoints, and private HTTP client.

### 2.2 `genai-rs`'s Adapter-Based Architecture

Instead of having each provider own its request loop, `genai` decouples *HTTP orchestration* from *API payload serialization*:

```rust
pub trait Adapter {
    fn default_auth() -> AuthData;
    fn default_endpoint() -> Endpoint;
    fn all_model_names(kind: AdapterKind) -> Result<Vec<String>>;
    fn get_service_url(model: &ModelIden, service_type: ServiceType, endpoint: Endpoint) -> Result<String>;
    fn to_web_request_data(
        model: &ModelIden,
        service_type: ServiceType,
        req: &ChatRequest,
        options: &ChatOptionsSet,
    ) -> Result<WebRequestData>;
    fn to_chat_response(web_res: WebResponse) -> Result<ChatResponse>;
    fn to_chat_stream(
        model_iden: ModelIden,
        reqwest_builder: RequestBuilder,
        options_set: ChatOptionsSet,
    ) -> Result<ChatStreamResponse>;
    // ...
}
```

* **Stateless Adapters:** Implementing structs do not own an HTTP client or execute async calls themselves. Instead, they translate a generic library request into `WebRequestData` (base URL, headers, and request body) or translate raw HTTP response text back into structured results.
* **Unified Client Execution:** The core `Client` owns the `reqwest::Client` connection pool, and executes requests:

```rust
// Unified client execution loop inside genai-rs
let web_request_data = adapter::to_web_request_data(model, service_type, req, options)?;
let response = self.web_client.execute(web_request_data).await?;
let chat_response = adapter::to_chat_response(response)?;
```

---

## 3. Detailed Tradeoff & Structural Analysis

### 3.1 Code Reusability & DRY Principles

* **CADE Custom:** Code duplicate is high. Every provider duplicate-implements standard HTTP response parsing, error status mapping, custom headers, and token counting logic.
* **`genai-rs`:** Adapters are extremely lightweight and pure. Adding a new OpenAI-compatible preset (e.g., DeepSeek, Groq, Together) is as simple as defining endpoint mapping and relying on the existing base `OpenAIAdapter`.

### 3.2 Feature-Specific Flexibility & Special Cases

CADE's custom architecture shines when accommodating complex, proprietary platform behaviors:

1. **Gemini Thinking Signatures:** CADE's `LlmToolCall` includes a `thought_signature` field which must be held in the message state and sent back verbatim during reasoning turns.
2. **GPT-5 / Responses API:** For newer reasoning/computer-use models, CADE handles structural routing changes seamlessly (switching endpoints from `/v1/chat/completions` to `/v1/responses` on-the-fly).
3. **Advanced Token Caching:** CADE's `TokenUsage` specifically handles Anthropic's prompt-caching headers (`cache_read_tokens`, `cache_write_tokens`) to ensure precise billing and cost tracking.

Integrating these proprietary edge-cases into `genai`'s rigid payload structs can be difficult without upstream library contributions or subclassing custom properties, introducing friction to the agent's core capabilities.

### 3.3 State Management & Hot-Reloading

CADE's `LlmRouter` acts as an active, mutable registry:

* **Dynamic Insertion/Removal:** Supports runtime modification of registered providers (`add_provider_with_key`, `remove_provider`) which allows the terminal repl (`/connect`) or DB config loads to hot-reload connected engines instantly.
* **Concurrent Model Probing:** Dynamically crawls active provider endpoints concurrently (e.g., querying local Ollama instance tags and OpenRouter /v1/models simultaneously) to build live model lists on-the-fly.

In `genai`, the client builder pattern is primarily intended for static initialization on app startup. While you can supply custom `AuthResolver` and `ServiceTargetResolver` callback functions to fetch keys dynamically from a datastore, hot-reloading whole providers and mapping them on-the-fly is less idiomatic.

---

## 4. Strategic Recommendations for CADE

Based on this deep architectural analysis, here are the strategic recommendations for the CADE project:

### Recommendation 1: Maintain the Custom `crates/cade-ai` Engine (Do Not Replace with `genai-rs`)

**Justification:** CADE is not a simple chat client; it is an autonomous agentic workspace. CADE's architecture is highly dependent on first-class, deep integrations of reasoning models (GPT-4.5/5, o-series, Gemini 2.5) with strict compliance requirements:
* CADE must handle **thought signature tracking** to ensure correct state loops during multi-turn tool execution.
* CADE's cost-and-pricing engine relies on **precise prompt caching token metrics** directly reported by Anthropic.
* CADE requires **unconditional, safe runtime provider-switching and hot-reloading** to support connection management seamlessly across different SSH/Local backends.

Replacing CADE's tailored types with `genai`'s generalized models would risk breaking these advanced agentic pipelines.

### Recommendation 2: Refactor CADE's Internal Engine to Adopt `genai-rs`'s Transform Decoupling

While CADE should keep its custom provider engine, it should refactor its internal structure to adopt the **best lessons of the `genai-rs` decoupled adapter pattern**:

1. **Centralize the HTTP Client:** Avoid initializing a new `reqwest::Client` in every single provider. Refactor `AiConfig` or `LlmRouter` to hold a single `Arc<reqwest::Client>` connection pool and pass it down.
2. **Standardize Request-Builder Utilities:** Extract shared request logic (such as retry-with-backoff, JSON error response parsing, and standard streaming chunk extraction) into a common `crates/cade-ai/src/utils.rs` module.
3. **OpenAI Compatibility Base Adapter:** Create a generic `OpenAiCompatProvider` struct that takes custom endpoints as arguments. This allows mapping and registering any OpenAI-compatible provider (Groq, Together, DeepSeek, OpenRouter) with zero code duplication, exactly like `genai`'s adapter reuse pattern.

---

## 5. Architectural Implementation Roadmap (Refactoring Guide)

### Refactoring Step 1: Centralized Connection Pooling

```rust
// Proposed LlmRouter structure for shared HTTP client
pub struct LlmRouter {
    providers: std::collections::HashMap<String, Arc<dyn LlmProvider>>,
    provider_keys: std::collections::HashMap<String, String>,
    default_provider: String,
    // Unified reqwest Client shared across all provider backends
    pub http_client: reqwest::Client,
    pub ollama_base_url: String,
}
```

### Refactoring Step 2: Unifying the OpenAI-Compatible Preset Providers

Currently, CADE constructs a custom provider for preset/OpenAI-compatible endpoints:

```rust
// Current Approach in router.rs
providers.insert(
    preset.name.clone(),
    Arc::new(openai::OpenAiProvider::new(
        key.clone(),
        Some(preset.chat_url.to_string()),
    )),
);
```

By standardizing a single `OpenAiCompatProvider` which supports dynamic overrides, we can encapsulate all OpenAI variations under a single, highly-optimized adapter class, allowing the base `OpenAiProvider` to stay focused on genuine first-party OpenAI endpoints.

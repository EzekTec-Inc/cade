# Implementation Plan: CADE Advanced Future Frontiers

This document outlines the architectural implementation plan to realize CADE's three Advanced Future Frontiers: Streaming Rig Models, Platform-Gated Sandbox Validation, and WASM Plugin Lifecycle Hooks.

---

## 1. Streaming Rig Models (`rig-compat`)

**Goal:** Extend CADE's `RigProviderAdapter` to support `rig`'s streaming completion interfaces, feeding token deltas and reasoning blocks directly into CADE's immediate-mode rendering engine.

*   [ ] **Step 1.1: Verify `rig` Streaming Traits**
    *   Inspect `rig_core::completion::CompletionModel::stream` and `StreamingCompletionResponse` using `context7`.
*   [ ] **Step 1.2: Implement `stream` Adapter in `rig_adapter.rs`**
    *   Update `LlmProvider::stream` implementation inside `crates/cade-ai/src/rig_adapter.rs` feature-gated behind `#[cfg(feature = "rig-compat")]`.
    *   Map `rig`'s `StreamingCompletionResponse` chunks (text and metadata) into CADE's native `StreamChunk` enum (e.g. mapping textual tokens to `StreamChunk::Text(delta)`).
*   [ ] **Step 1.3: Connect Abort and Cancellation**
    *   Integrate CADE's thread-safe atomic `SafeAbortHandle` inside the streaming future to ensure that cancelling the stream immediately terminates `rig`'s underlying fetch stream.

---

## 2. Platform-Gated Sandbox Validation (`crates/cade-agent`)

**Goal:** Evolve `SandboxManager` to actively run security profiles and automatically scan command tokens for malicious shell injection before dispatching them to Docker/SSH environments.

*   [ ] **Step 2.1: Establish Security Profile Schema**
    *   Define a `SecurityProfile` enum (e.g. `Low`, `Standard`, `Strict`) inside CADE's configuration settings.
*   [ ] **Step 2.2: Implement Lexical Token Scanner**
    *   Add a shell-command scanner inside `crates/cade-agent/src/backends/mod.rs` (using `shlex` or a custom tokenizer) to inspect commands before execution.
    *   Block known malicious shell injection patterns (like `&& rm -rf /`, `curl ... | sh`, `$(...)`, or backtick command substitutions) when running under `Strict` profile, raising an explicit `SecurityException`.
*   [ ] **Step 2.3: Intercept and Ask in TUI**
    *   If a command is flagged under `Standard` profile, halt the execution future and trigger CADE's active `ask_permission` context overlay, prompting the user for explicit confirmation before spawning the subshell.

---

## 3. WASM Plugin Lifecycle Hooks (`crates/cade-agent/src/plugins/`)

**Goal:** Create a runtime event loop inside the WebAssembly container allowing third-party WASM tools to subscribe to and handle conversation update channels natively.

*   [ ] **Step 3.1: Define Lifecycle Event Schemas**
    *   Define a serialization structure for CADE lifecycle events (e.g. `Event::MessageSent`, `Event::ToolExecuted`, `Event::MemoryUpdated`) in `crates/cade-plugin/src/types.rs`.
*   [ ] **Step 3.2: Implement Guest-Host Event Bus**
    *   Add a shared event dispatcher inside `crates/cade-agent/src/plugins/wasm_runtime.rs` that maintains a list of registered WASM listener exports.
    *   Whenever CADE commits a message or updates a memory block, post the serialized event payload into the WASM linear memory and trigger the guest's callback.
*   [ ] **Step 3.3: Expose State Query Bridges**
    *   Expose safe host import functions to the WASM guest, allowing sandboxed plugins to query active conversation history, read memory blocks, and submit background actions dynamically.

---

## 4. Verification & Testing

*   [ ] **Step 4.1: Workspace Compilations**
    *   Verify clean, warning-free compile states across all configurations:
        `cargo check --workspace`
        `cargo check --workspace --features rig-compat`
*   [ ] **Step 4.2: TDD Validation**
    *   Write robust, behavior-focused unit and integration tests verifying the stream adapter mappings, shell injection blocks, and WASM event triggers under separate RED-GREEN cycles.
    *   Run and pass all tests:
        `cargo test --workspace`

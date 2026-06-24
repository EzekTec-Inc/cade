# Implementation Plan: Optional Rig-Core Compatibility Adapters in CADE

This document outlines the architectural plan to integrate optional, feature-gated `rig-core` compatibility adapters in CADE. This allows CADE to instantly support any LLM provider and vector database from the `rig` ecosystem while keeping CADE's core system, immediate-mode TUI, and security sandboxes completely native and low-risk.

---

## 1. Feature Flag Configuration (`rig-compat`)

**Goal:** Keep `rig-core` completely optional. It will compile only when the `rig-compat` feature flag is explicitly enabled during the build.

*   [ ] **Step 1.1: Configure `crates/cade-ai/Cargo.toml`**
    *   Add `rig-compat` under `[features]`.
    *   Add `rig-core = { version = "0.6", optional = true }` under `[dependencies]`.
*   [ ] **Step 1.2: Configure `crates/cade-store/Cargo.toml`**
    *   Add `rig-compat` under `[features]`.
    *   Add `rig-core = { version = "0.6", optional = true }` under `[dependencies]`.
*   [ ] **Step 1.3: Wire to workspace root `Cargo.toml`**
    *   Expose `rig-compat` in the root manifest, forwarding the feature-gate down to `cade-ai` and `cade-store` workspace members.

---

## 2. Dynamic Model Connectors (`crates/cade-ai/src/rig_adapter.rs`)

**Goal:** Wrap `rig_core`'s community-driven `CompletionModel` traits inside CADE's `LlmProvider` interface, allowing CADE to consume any `rig` provider out of the box.

*   [ ] **Step 2.1: Implement `RigProviderAdapter`**
    *   Create `crates/cade-ai/src/rig_adapter.rs` feature-gated behind `#[cfg(feature = "rig-compat")]`.
    *   Declare `struct RigProviderAdapter<M: CompletionModel> { pub model: M }`.
*   [ ] **Step 2.2: Map completion requests**
    *   Implement `LlmProvider` for `RigProviderAdapter`.
    *   Serialize CADE's `CompletionRequest` (prompt history, message content, system directives) into `rig`s input structures.
    *   Format `rig`s `ModelResponse` back into CADE's native `CompletionResponse` signature.
*   [ ] **Step 2.3: Register in CADE provider router**
    *   Register the new adapter in `crates/cade-ai/src/router.rs` so it is selectable as a dynamic endpoint.

---

## 3. Enterprise Vector Database Drivers (`crates/cade-store/src/sqlite/rig_store.rs`)

**Goal:** Wrap CADE's local SQLite vector index and other `rig` database drivers under CADE's `VectorIndex` interface to allow seamless cloud/enterprise scaling.

*   [ ] **Step 3.1: Expose CADE SQLite as `rig_core::Result`**
    *   Create `crates/cade-store/src/sqlite/rig_store.rs` feature-gated behind `#[cfg(feature = "rig-compat")]`.
    *   Implement `rig_core::vector_store::Result` for CADE's local connection-pooled SQLite index, allowing other `rig` applications to read/write CADE's database.
*   [ ] **Step 3.2: Implement `RigStoreAdapter` in CADE**
    *   Implement CADE's internal `VectorIndex` trait for any external `rig` vector store driver (e.g. Qdrant, pgvector, MongoDB).
    *   Allow the user to configure external cloud databases in `settings.json` and query them seamlessly using CADE's local RAG engine.

---

## 4. Verification & Testing

*   [ ] **Step 4.1: Compilation and Warnings**
    *   Verify the project compiles cleanly under the default feature sets with no warnings:
        `cargo check --workspace`
    *   Verify compilation succeeds with `rig-compat` enabled:
        `cargo check --workspace --features rig-compat`
*   [ ] **Step 4.2: Unit Tests**
    *   Create unit tests under `crates/cade-ai/src/rig_adapter.rs` and `crates/cade-store/src/sqlite/rig_store.rs` validating serialization, search routing, and exception maps.
    *   Run and verify all workspace tests pass:
        `cargo test --workspace`

# Implementation Plan: Emulating Opencode High-Impact Capabilities in CADE

This document outlines the architectural strategy to design, build, and integrate three advanced capabilities derived from our deep `opencode` investigation: Declarative TUI signals, a JS/TS/WASM Runtime Plugin System, and Active Runtime Permission Gating.

---

## 1. Declarative TUI Signals (Ratatui Core Extension)
**Goal:** Minimize CPU overhead and simplify state management by introducing declarative signals (similar to SolidJS) over CADE's Immediate-Mode Ratatui renderer.

*   [ ] **Step 1.1: Introduce State Signals**
    *   Create a `Signal<T>` wrapper struct in `crates/cade-tui/src/signals.rs` utilizing `tokio::sync::watch` for thread-safe state synchronization.
    *   Expose `.read()` to acquire immutable snapshots and `.write(val)` to update the state and automatically trigger `draw_dirty = true`.
*   [ ] **Step 1.2: Refactor Components to Render on Signal Fire**
    *   Refactor the active viewports (e.g., footer, sidebar, timeline) to subscribe to corresponding signals.
    *   Optimize the event tick loop in `read_input` to skip rendering entirely unless a signal's dirty flag is raised, dropping idle render CPU cycles to ~0%.

---

## 2. Dynamic Plugin System (rquickjs TS/JS & Wasmer WASM Runtimes)
**Goal:** Expose an extensible plugin system allowing third-party developers to author tools in JavaScript/TypeScript or safe WASM binaries.

*   [ ] **Step 2.1: Integrated Javascript Runtime (rquickjs)**
    *   In `crates/cade-agent/Cargo.toml`, add `rquickjs` as an optional, feature-gated dependency (`features = ["plugin-js"]`).
    *   Create `crates/cade-agent/src/plugins/js_runtime.rs` to initialize a safe QuickJS context. Expose standard CADE tools (filesystem, prompt, ask_user) as native JS functions inside the environment.
*   [ ] **Step 2.2: Integrated WebAssembly Sandbox (Wasmer / Wasmtime)**
    *   Create `crates/cade-agent/src/plugins/wasm_runtime.rs` to load and execute `.wasm` files inside a strict WASI sandbox.
*   [ ] **Step 2.3: Zod-to-JsonSchema Tool Loader**
    *   Provide a standardized JSON schema parser inside `crates/cade-agent/src/tools/manager.rs` that loads runtime-configured JS/WASM plugin schemas and maps them dynamically.

---

## 3. Active Runtime Permission Gating
**Goal:** Evolve CADE's static pre-execution permission checker into an active runtime evaluator where running tools can programmatically request user permissions for sub-actions mid-execution.

*   [ ] **Step 3.1: Context-Driven Dynamic Querying**
    *   Enhance CADE's `ToolContext` trait in `crates/cade-core/src/capabilities/` or `crates/cade-agent/src/tools/traits.rs` to expose an async callback:
        ```rust
        async fn ask_permission(&self, permission: &str, pattern: &str) -> bool;
        ```
*   [ ] **Step 3.2: TUI Future Interceptor & Modal Prompt**
    *   Update `crates/cade-tui/src/app/input.rs` and the parallel tool execution runner. When a tool calls `ask_permission`, halt the execution future.
    *   Push an interactive `PermissionOverlay` modal onto CADE's TUI overlay stack.
    *   Once the user approves/denies, resume the tool's execution future with the verdict.
*   [ ] **Step 3.3: Dynamic Bash Command Interception**
    *   Connect the active permission evaluator to CADE's internal `bash` execution tool. If a shell command spawns nested sub-commands (e.g. executing `make` which runs compilers), intercept and evaluate each nested token programmatically.

---

## Verification & Quality Gates
- Compile with zero warnings: `cargo clippy --workspace -- -D warnings`
- Verify formatting compliance: `cargo fmt --all -- --check`
- Execute complete workspace tests: `cargo test --workspace`

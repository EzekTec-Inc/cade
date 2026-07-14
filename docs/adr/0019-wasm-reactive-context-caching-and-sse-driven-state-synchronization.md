# ADR 19: WASM Reactive Context Caching and SSE-Driven State Synchronization

* **Status**: Accepted
* **Decided on**: 2026-07-13

## Context

`cade-gui` is a web dashboard compiled to WebAssembly (WASM) using Dioxus 0.5. Currently, it suffers from three major efficiency and effectiveness bottlenecks:
1. **Network Inefficiency**: The client relies on periodic polling (every 10 seconds) to fetch updated conversation lists and agent metadata. This wastes user bandwidth and introduces lag before background agent events are updated.
2. **Heavy Render Overhead**: On every streaming message update, the entire timeline is re-evaluated, text-parsed, and re-rendered into the browser's DOM tree. In long sessions, this causes noticeable WASM render spikes, layout thrashing, and sluggish scrolling.
3. **State Loss on Navigation**: Switching views (e.g., from Chat to Settings) unmounts components, purging raw message structures and forcing complete refetching of all assets from the server.

## Decision

We decided to implement a high-performance **WASM Reactive Context Cache** and a real-time **SSE Event Dispatch Loop** inside the `cade-gui` client:

### 1. Persistent SSE-Driven State Synchronization
* Establish a single, persistent Server-Sent Events (SSE) stream at the application root (`App()` component) connecting to the server events endpoint.
* All incoming server updates are processed in a global background task, dispatching and updating Dioxus signals (`Signal<T>`) reactively without periodic polling loops.

### 2. Global Component-Level Memoization (`ParsedMessageCache`)
* Decouple message HTML parsing and `<reasoning>` block splitting from active render cycles. Heavy computations are performed only once upon message completion or initial load.
* Memoize completed message components to prevent Dioxus from triggering re-evaluation or layout calculations on completed messages during live stream deltas.

### 3. Global AppState Cache Persistence
* Store the memoized/parsed messages in-memory inside the global, session-scoped `AppState` context. 
* This ensures that cache structures survive view switching or page navigation without unmounting or triggering network reload penalties.

## Consequences

### Positive (Pros)
* **Instantaneous Feedback**: Eliminates 10-second polling delays, delivering real-time agent updates instantly to the GUI.
* **Drastic CPU Reduction**: Message memoization completely eliminates rendering overhead on completed messages, keeping the chat buttery smooth even during massive code streams.
* **Zero Network Waste**: The UI only refetches data when explicitly needed, saving significant network and server resources.

### Negative (Cons)
* **Memory Overhead**: Storing parsed message caches in global WASM memory increases overall client memory footprint slightly, though this is negligible compared to standard DOM structures.

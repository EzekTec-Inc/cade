# ADR 17: Asynchronous Queue-Decoupled Lua Plugins for TUI Responsiveness

* **Status**: Accepted
* **Decided on**: 2026-07-04

## Context

The embedded Lua scripting engine in `cade-tui` enables dynamic extensibility, allowing users and plugins to inject custom widgets, register slash commands, and bind global keybindings. 

However, because both the TUI render loop and the Lua interpreter run on the primary main thread, any blocking operation executed within a Lua plugin (e.g., synchronous file access, network requests, or long-running computations) directly freezes the TUI event loop. This leads to stuttering animations, laggy keystroke handling, and unresponsive typewriter reveals.

## Decision

We decided to strictly enforce an **Asynchronous Queue-Decoupled Architecture** for all Lua plugin interactions:

### 1. Prohibition of Synchronous Blocking Rust Callouts
* Lua script handlers are strictly prohibited from executing blocking, synchronous operations on the main TUI thread. 
* All heavy native actions (filesystem queries, tool executions, background agent tasks) must be offloaded as asynchronous payloads.

### 2. Queue-Based Interaction Seams (`command_queue` and `tool_queue`)
* When a Lua widget or callback triggers an action, it writes to non-blocking thread-safe queues:
  * `command_queue`: Queue for registering slash commands to be dispatched in the background.
  * `tool_queue`: Queue for requesting tool execution from the host.
* The Rust side polls these queues, processes the requested tasks in background worker threads, and keeps the TUI rendering loop clean and responsive.

### 3. Asynchronous UI Event Callbacks (`ui_event_queue` / `_ui_callbacks`)
* Upon completing background computations, the Rust host posts a JSON-serialized event back to `ui_event_queue`.
* During its idle tick, the TUI event loop pops events from `ui_event_queue` and invokes the registered Lua callback function via `handle_ui_event`.
* This ensures that all data flow between Lua and the native system is asynchronous and event-driven.

## Consequences

### Positive (Pros)
* **Smooth UI Performance**: Guarantees the TUI main thread remains completely fluid at all times, preserving smooth scrolling, animations, and high responsiveness.
* **Safer Scripting Sandbox**: Decoupling prevents malicious or poorly-written plugins from easily locking up the entire terminal interface.
* **Unified State Management**: Standardizes the protocol for plugin-to-host and host-to-plugin communication via structured, serializable JSON messages.

### Negative (Cons)
* **API Complexity**: Plugin developers must write asynchronous, callback-driven Lua scripts rather than straightforward, sequential synchronous code.


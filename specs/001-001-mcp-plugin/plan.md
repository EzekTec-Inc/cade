# Implementation Plan: MCP-UI TUI Plugin for CADE

**Branch**: `001-mcp-ui-plugin` | **Date**: 2026-05-17

**Input**: Feature specification from `/specs/001-001-mcp-plugin/spec.md`

## Summary

Implement an `mcp-ui` Host inside CADE using a Lua plugin. The plugin will intercept MCP tool results containing `_meta.ui.resourceUri`, fetch the corresponding UI payloads (e.g., HTML or structured layout schemas), and transpile them into native `LuaWidget` components (like Paragraphs, Buttons, and Toggles) to be rendered dynamically in CADE's sidebar.

## Technical Context

**Language/Version**: Lua 5.1 (Luau/CADE environment) and Rust 1.70+

**Primary Dependencies**: `cjson` (for parsing UI payloads), `curl` or Rust exposed HTTP fetchers, and the existing `LuaWidget` enum in `cade-tui`.

**Storage**: In-memory state tracking for active MCP UI components.

**Testing**: Manual TUI visual testing and mocked MCP server responses.

**Target Platform**: Local Terminal / CADE TUI.

**Project Type**: Lua Plugin for Rust Host.

**Performance Goals**: UI transpilation and rendering must occur seamlessly without blocking the main TUI render loop.

**Constraints**: Terminals cannot render true DOM, so HTML transpilation is limited to basic text styling, lists, and layout approximation.

**Scale/Scope**: 1 Lua file (`mcp_ui_host.lua`), potentially exposing an HTTP hook from Rust if native Lua HTTP is insufficient.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] Does not introduce panic-prone Rust code (Pure Lua implementation).
- [x] Adheres to TUI rendering constraints (utilizes existing `LuaWidget` mappings).
- [x] Fosters architectural isolation (plugin logic stays entirely in `.cade/plugins/`).

## Project Structure

### Documentation (this feature)

```text
specs/001-mcp-ui-plugin/
├── plan.md              # This file
├── spec.md              # Requirements and User Stories
└── tasks.md             # Breakdown of actionable tasks
```

### Source Code (repository root)

```text
.cade/
└── plugins/
    └── mcp_ui_host.lua  # The core transpiler and MCP interceptor

crates/
└── cade-tui/
    └── src/
        └── lua_ui.rs    # (Only touched if we need to expose an HTTP fetcher to Lua)
```

**Structure Decision**: A single Lua plugin file inside `.cade/plugins/` is the optimal approach, maintaining perfect separation of concerns from the core CADE binaries.

## Tasks & Phases

1. **Phase 1: Interception Logic**
   - Implement `CADE.hook_mcp_result` (or equivalent) in `mcp_ui_host.lua`.
   - Detect the `_meta.ui.resourceUri` field in the JSON payload.
2. **Phase 2: Fetch & Transpile**
   - Write a Lua function to fetch the UI payload.
   - Write an HTML/Schema-to-LuaWidget transpiler function (`transpile_to_widgets`).
3. **Phase 3: Render & Interact**
   - Update `CADE_UI.sidebar` dynamically with the transpiled widgets.
   - Bind click events back to the MCP server's state update endpoints.

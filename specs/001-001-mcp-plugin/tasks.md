# Implementation Tasks: MCP-UI TUI Plugin for CADE

**Branch**: `001-mcp-ui-plugin`
**Spec**: `/specs/001-001-mcp-plugin/spec.md`
**Plan**: `/specs/001-001-mcp-plugin/plan.md`

---

## Task 1: Setup Plugin Skeleton and Hook Interception
**Status**: Done
**Description**: Create the Lua plugin file and implement the logic to intercept MCP tool results.

- [x] Create `.cade/plugins/mcp_ui_host.lua`.
- [x] Implement an interception mechanism to parse outgoing/incoming MCP messages (e.g., using a global event hook if CADE provides one, or monkey-patching the UI update cycle).
- [x] Write logic to detect `_meta.ui.resourceUri` inside the result JSON.

## Task 2: Implement HTTP Fetcher in Lua
**Status**: Done
**Description**: Create a secure way for the Lua script to fetch the UI resource from the MCP server.

- [x] Determine if CADE exposes a native HTTP fetcher to Lua. If not, implement a basic wrapper around `curl` using `os.execute` or `io.popen`.
- [x] Fetch the payload from the `resourceUri` and decode the JSON response.

## Task 3: HTML to LuaWidget Transpiler
**Status**: Done
**Description**: Write the core parsing logic that converts `rawHtml` or MCP App schemas into CADE `LuaWidget` tables.

- [x] Write `function transpile_to_widgets(payload)`.
- [x] Map basic text (`<h1>`, `<p>`) to `type = "paragraph"`.
- [x] Map interactive elements (`<button>`) to `type = "button"` with unique IDs.
- [x] Map lists (`<ul>`, `<li>`) to `type = "list"`.
- [x] Implement stripping for unsupported HTML tags to prevent rendering garbage text.

## Task 4: Dynamic Rendering & Interactivity
**Status**: Done
**Description**: Bind the transpiled widgets to the CADE TUI and handle mouse clicks.

- [x] Assign the transpiled widgets to a `LuaWidget::Popup` or inject them into the main terminal viewport (`CADE_UI.main` or equivalent).
- [x] Use `CADE.bind_ui_callback` to listen for clicks on the newly generated buttons.
- [x] When a button is clicked, trigger the corresponding action defined in the MCP UI payload (e.g., executing another MCP tool or sending a state update request).
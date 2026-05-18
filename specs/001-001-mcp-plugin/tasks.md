# Implementation Tasks: MCP-UI TUI Plugin for CADE

**Branch**: `001-mcp-ui-plugin`
**Spec**: `/specs/001-001-mcp-plugin/spec.md`
**Plan**: `/specs/001-001-mcp-plugin/plan.md`

---

## Task 1: Setup Plugin Skeleton and Hook Interception
**Status**: Todo
**Description**: Create the Lua plugin file and implement the logic to intercept MCP tool results.

- [ ] Create `.cade/plugins/mcp_ui_host.lua`.
- [ ] Implement an interception mechanism to parse outgoing/incoming MCP messages (e.g., using a global event hook if CADE provides one, or monkey-patching the UI update cycle).
- [ ] Write logic to detect `_meta.ui.resourceUri` inside the result JSON.

## Task 2: Implement HTTP Fetcher in Lua
**Status**: Todo
**Description**: Create a secure way for the Lua script to fetch the UI resource from the MCP server.

- [ ] Determine if CADE exposes a native HTTP fetcher to Lua. If not, implement a basic wrapper around `curl` using `os.execute` or `io.popen`.
- [ ] Fetch the payload from the `resourceUri` and decode the JSON response.

## Task 3: HTML to LuaWidget Transpiler
**Status**: Todo
**Description**: Write the core parsing logic that converts `rawHtml` or MCP App schemas into CADE `LuaWidget` tables.

- [ ] Write `function transpile_to_widgets(payload)`.
- [ ] Map basic text (`<h1>`, `<p>`) to `type = "paragraph"`.
- [ ] Map interactive elements (`<button>`) to `type = "button"` with unique IDs.
- [ ] Map lists (`<ul>`, `<li>`) to `type = "list"`.
- [ ] Implement stripping for unsupported HTML tags to prevent rendering garbage text.

## Task 4: Dynamic Rendering & Interactivity
**Status**: Todo
**Description**: Bind the transpiled widgets to the CADE TUI and handle mouse clicks.

- [ ] Assign the transpiled widgets to a `LuaWidget::Popup` or inject them into `CADE_UI.sidebar`.
- [ ] Use `CADE.bind_ui_callback` to listen for clicks on the newly generated buttons.
- [ ] When a button is clicked, trigger the corresponding action defined in the MCP UI payload (e.g., executing another MCP tool or sending a state update request).
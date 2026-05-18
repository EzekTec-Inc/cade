# Feature Specification: MCP-UI TUI Plugin for CADE

**Feature Branch**: `001-mcp-ui-plugin`

**Created**: 2026-05-17

**Status**: Draft

**Input**: User description: "implement mcp-ui as a lua plugin for CADE to render MCP UI elements natively in the terminal"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Transpile Raw HTML to TUI (Priority: P1)

As a CADE user, when an MCP tool returns a UI resource URI (e.g., `_meta.ui.resourceUri`), the CADE Lua plugin should intercept this, fetch the `rawHtml`, and convert standard tags (`<h1>`, `<p>`, `<b>`) into formatted TUI elements (e.g., `LuaWidget::Paragraph` with styling).

**Why this priority**: HTML transpilation is the baseline requirement for bridging web-centric MCP tools to a terminal UI.

**Independent Test**: Can be tested by returning a mocked MCP tool response containing basic HTML and verifying it renders in the CADE sidebar.

**Acceptance Scenarios**:

1. **Given** an MCP tool returns `<h1>Widget</h1>`, **When** intercepted, **Then** CADE renders a styled bold cyan paragraph containing "Widget".
2. **Given** the resource contains unrecognized tags, **When** intercepted, **Then** the plugin strips the tags and displays the text gracefully.

---

### User Story 2 - Render Interactive Elements (Priority: P2)

As a CADE user, when an MCP UI resource contains interactive elements (like buttons or toggles), the plugin should map them to `LuaWidget::Button` and `LuaWidget::Toggle` so I can interact with them.

**Why this priority**: Interactivity is the core value proposition of `mcp-ui`.

**Independent Test**: Can be tested by fetching an MCP UI resource representing a form and ensuring clicking the buttons triggers a state change in the TUI.

**Acceptance Scenarios**:

1. **Given** an MCP UI resource has a button, **When** the plugin parses it, **Then** it generates a `LuaWidget::Button` that can be clicked using CADE's mouse handling.

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST intercept MCP tool results containing `_meta.ui.resourceUri`.
- **FR-002**: System MUST fetch the UI payload from the MCP server using the provided URI.
- **FR-003**: System MUST parse the incoming JSON and translate recognized HTML or UI schema into `LuaWidget` representations.
- **FR-004**: System MUST render the resulting widgets inside CADE's sidebar or as a `LuaWidget::Popup`.
- **FR-005**: System MUST route user interactions (e.g., button clicks) back to the corresponding MCP tool or state update mechanism.

### Key Entities 

- **MCP App Host**: The Lua script acting as the intermediary.
- **LuaWidget Tree**: The mapped representation of the web UI converted into TUI components.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Extracted `mcp-ui` elements render in the CADE TUI within 500ms of the MCP tool execution completing.
- **SC-002**: Buttons and toggles defined in the MCP UI resource correctly register interactions via `CADE.bind_ui_callback`.

## Assumptions

- The `mcp-ui` MCP servers are running locally or are accessible without complex external authentication.
- CADE's Lua environment has access to basic JSON parsing (`cjson` or equivalent) and network fetching capabilities (via exposed Rust hooks or `curl`).
- Complex React/JS logic sent by MCP servers is out of scope for V1; we are only parsing static layout schemas or HTML strings returned by the UI resource.

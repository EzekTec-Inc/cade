# CADE UI/UX Polish & Enhancement Implementation Plan

This document outlines the detailed architectural design, target files, implementation stages, and live tracking status of CADE's UI/UX polish initiatives.

---

## 1. Tracking Dashboard

| Phase | Feature | Target Files | Status |
| :--- | :--- | :--- | :--- |
| **TUI-1** | Interactive Code-Block Folding | `crates/cade-tui/src/app/timeline/mod.rs`, `markdown.rs` | 🟢 Planned |
| **TUI-2** | Relative Anchor-Retaining Resize | `crates/cade-tui/src/app/mod.rs`, `render.rs` | 🟢 Planned |
| **TUI-3** | Floating Slash-Command Autocomplete | `crates/cade-tui/src/app/command_palette.rs`, `autocomplete.rs` | 🟢 Planned |
| **TUI-4** | Live Budget & Cost Gauge | `crates/cade-tui/src/app/layout/sidebar.rs` | ✅ Complete |
| **TUI-5** | Toast Decay Progress Bars | `crates/cade-tui/src/app/layout/toast.rs` | ✅ Complete |
| **GUI-1** | High-Fidelity Network Node Graphs | `crates/cade-gui/src/app/` | 🟢 Planned |

*Status Indicators: 🟢 Planned | 🟡 In Progress | ✅ Complete*

---

## 2. Detailed Feature Designs

### TUI-1: Interactive Code-Block Folding
*   **Problem**: Heavy log files or 500-line JSON blocks flood the conversation history.
*   **Design**:
    *   Introduce `folded_blocks: HashSet<String>` inside `TuiApp` or `TimelineState` representing unique identifiers for code block regions (derived from block hash/location).
    *   In `parse_markdown_lines_with_theme`, append dynamic fold/unfold text badges to code blocks (e.g. `[f] Expand / Collapse`).
    *   When the cursor is focused, hitting `f` toggles the identifier's membership in `folded_blocks`, prompting an immediate layout recount and viewport redraw.
*   **Target Files**:
    *   `crates/cade-tui/src/markdown.rs`
    *   `crates/cade-tui/src/app/timeline/mod.rs`

### TUI-2: Relative Anchor-Retaining Resize
*   **Problem**: Terminal window resizes recalculate wrapping offsets, causing the scroll position to "jump" or lose the user's active reading context.
*   **Design**:
    *   Add `anchor_message_key: Option<TimelineKey>` inside `TimelineState` representing the top-most fully visible message prior to resize.
    *   On terminal resize events (`Event::Resize`), capture the anchor message.
    *   After reflow, compute the pre-wrapped rows preceding `anchor_message_key` and automatically update the scroll offset to align the viewport precisely with this message.
*   **Target Files**:
    *   `crates/cade-tui/src/app/mod.rs`
    *   `crates/cade-tui/src/app/render.rs`

### TUI-3: Floating Slash-Command Autocomplete Menu
*   **Problem**: Slash commands (`/skills`, `/theme`, `/permissions`) are invisible to standard typing until `Tab` is explicitly hit.
*   **Design**:
    *   Modify `TuiApp::draw` to monitor input text. If the input buffer starts with `/`, construct a floating mini-list overlay directly above the text input bounds.
    *   Include a short summary card/badge next to each command to explain its action.
    *   Ensure Up/Down arrows traverse this floating list when active, and `Enter` inserts the selected command into the buffer.
*   **Target Files**:
    *   `crates/cade-tui/src/app/command_palette.rs`
    *   `crates/cade-tui/src/autocomplete.rs`

### TUI-4: Live Budget & Cost Gauge
*   **Problem**: Spend tracking (`CADE_MAX_SESSION_COST_USD`) is a passive background guardrail.
*   **Design**:
    *   Retrieve the current session's cumulative cost and maximum dollar limit from `AgentMetrics` / `AppState` on each turn.
    *   Render a high-fidelity visual horizontal gauge in the sidebar.
    *   Style the gauge dynamically: Green for `cost < 50%`, Yellow for `50% <= cost < 85%`, and Red/Blinking for `cost >= 85%` budget limit.
*   **Target Files**:
    *   `crates/cade-tui/src/app/layout/sidebar.rs`

### TUI-5: Toast Decay Progress Bars
*   **Problem**: Toasts pop off the interface abruptly without visually signaling their lifetime.
*   **Design**:
    *   Add `started_at: Instant` and `ttl: Duration` to each toast item.
    *   When rendering the toast, compute the percentage of lifetime remaining:
        $$\text{pct\_remaining} = \frac{\text{ttl} - \text{started\_at.elapsed()}}{\text{ttl}}$$
    *   Draw a thin progress sub-border or horizontal fill line on the bottom edge of the toast box matching `pct_remaining`.
*   **Target Files**:
    *   `crates/cade-tui/src/app/layout/toast.rs`

---

## 3. Next Steps & Phase Alignment

1.  **Review & Signoff**: Share this implementation and tracking plan with the team.
2.  **Modular Development**: Tackle each feature sequentially following strict Test-Driven Development (TDD) principles.
3.  **Verification**: Conduct real-world user interface validation on each implemented item and update the tracking status above.

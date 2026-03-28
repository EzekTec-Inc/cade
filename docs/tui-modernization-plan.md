# CADE TUI Modernization Plan

## Objective

Modernize CADE’s terminal UI while keeping the current Rust-only, low-overhead stack:

- keep `ratatui` + `crossterm`
- preserve the single fullscreen render path in `crates/cade-tui`
- avoid framework rewrites or large new dependencies
- improve layout, hierarchy, polish, and interaction without regressing streaming performance

## Design Constraints

1. **No framework swap**
   - Do not replace `ratatui` / `crossterm`.
   - Do not introduce Electron, browser UI, or a second UI stack.

2. **Keep the current runtime model**
   - `TuiApp` remains the root fullscreen app.
   - Streaming remains throttled (`DRAW_MIN_INTERVAL`) and non-blocking.
   - Existing REPL / tool orchestration should continue to work during UI changes.

3. **Minimize dependency overhead**
   - Prefer extending existing crates over adding new ones.
   - Only add dependencies if they meaningfully reduce complexity.

4. **Ship incrementally**
   - Each phase should be independently useful and releasable.
   - Do not block early UX improvements on the later timeline refactor.

---

## Success Criteria

The modernization is successful if CADE gains:

- a more app-like terminal layout
- clearer visual distinction between assistant text, tool calls, tool results, reasoning, and system output
- a more informative composer/input area
- cleaner, consistent overlays
- optional sidebar/status surfaces that reduce footer overload
- better theme consistency
- no meaningful regression in responsiveness, scroll behavior, or streaming stability

---

## Non-Goals

- Building a browser UI
- Rewriting CADE into a component framework beyond current `ratatui`
- Reworking agent logic or REPL flow unless required for TUI integration
- Implementing every possible command palette/sidebar feature in the first pass

---

## Current State Summary

Relevant files:

- `crates/cade-tui/src/app.rs`
  - main fullscreen renderer
  - layout, status/footer, content rendering, overlays
- `crates/cade-tui/src/editor.rs`
  - input editor, undo/redo, paste handling, input mode detection
- `crates/cade-tui/src/colors.rs`
  - theme tokens already exist
- `crates/cade-tui/src/markdown.rs`
  - markdown and syntax highlighting
- `crates/cade-tui/src/menu.rs`
- `crates/cade-tui/src/skills.rs`
- `crates/cade-tui/src/session_tree.rs`
- `crates/cade-core/src/settings/manager.rs`
  - future home for TUI settings
- `src/main.rs`
  - theme loading / TUI wiring

Key architectural limitation today:

- the conversation viewport is still primarily a **flattened `RenderLine -> Vec<Line> -> Paragraph` pipeline**
- this is efficient, but limits rich block/card rendering, per-item focus, and future interaction density

---

# Phase 0 — Baseline, inventory, and guardrails

## Goal
Establish a safe baseline before visual refactors.

## Tasks

1. **Capture current behavior**
   - record screenshots/gifs of:
     - idle screen
     - streaming assistant output
     - tool call + tool result sequences
     - `/help`, `/skills`, `/tree` overlays
     - long markdown output
     - reasoning block
     - queued input while agent is busy
   - use these as regression references

2. **Identify remaining hardcoded color usage**
   - audit `app.rs`, `markdown.rs`, `menu.rs`, `skills.rs`, `session_tree.rs`
   - list all places still bypassing `ThemeColors`

3. **Measure current interaction invariants**
   - scroll follow behavior
   - `Ctrl+O` expand behavior
   - queue badge behavior
   - viewport stability during stream commit
   - resize handling

4. **Define screen-width breakpoints**
   - recommended starting breakpoints:
     - `< 110 cols`: single-column mode
     - `>= 110 cols`: sidebar-capable mode

## Validation

- `cargo test -p cade-tui --lib`
- `cargo check -p cade-tui -p cade-cli`

---

# Phase 1 — Theme/token cleanup and visual foundation

## Goal
Make the current UI visually consistent before introducing new layout surfaces.

## Tasks

### 1.1 Expand `ThemeColors`
File: `crates/cade-tui/src/colors.rs`

Add tokens for:

- sidebar background / border
- toast background / border
- badge background / foreground
- assistant card accent / background
- reasoning card background
- system message tint
- overlay background / title / selection
- divider active / divider inactive
- input placeholder / input prefix / input badge colors

### 1.2 Remove hardcoded colors from `app.rs`
File: `crates/cade-tui/src/app.rs`

Replace remaining ad-hoc `RC::Rgb(...)` usage where practical with `ThemeColors`.

Priority areas:

- tool call markers
- user separator styling
- status spinner colors
- plan panel colors
- footer labels
- context severity colors where tokenized variants make sense

### 1.3 Make markdown theme-aware
File: `crates/cade-tui/src/markdown.rs`

Refactor markdown rendering so it can use resolved theme colors instead of hardcoded values.

Recommended approach:

- introduce a small `MarkdownTheme` derived from `ThemeColors`
- update `parse_markdown_lines(...)` to accept a theme parameter, or add a themed variant
- keep the current visual defaults as the fallback

### 1.4 Define shared overlay chrome
Files:
- `crates/cade-tui/src/menu.rs`
- `crates/cade-tui/src/skills.rs`
- `crates/cade-tui/src/session_tree.rs`
- possibly a new helper module under `crates/cade-tui/src/`

Extract a shared modal/overlay style helper for:

- border style
- title row
- dimmed background wash
- hint/footer row
- selected item styling

## Acceptance Criteria

- custom themes affect more of the UI consistently
- overlays look like one coherent design system
- no visual regressions in content rendering

## Validation

- `cargo test -p cade-tui --lib`
- manual check with built-in dark/light themes and one custom theme

---

# Phase 2 — Layout modernization: sidebar, composer badges, toasts

## Goal
Make the TUI feel more like a modern terminal application without changing the core render model yet.

## Tasks

### 2.1 Add responsive right sidebar
File: `crates/cade-tui/src/app.rs`

Add a width-aware horizontal split:

- narrow mode: current single-column layout
- wide mode: main content + right sidebar

Recommended sidebar sections:

1. **Session**
   - agent name
   - model
   - reasoning effort
   - cwd

2. **Status**
   - permission mode
   - context %
   - queued messages
   - streaming/thinking state

3. **Plan/Todos**
   - compact active-plan summary
   - if full plan panel is visible, sidebar can show only summary/status

4. **Hints**
   - key shortcuts (`Ctrl+O`, follow, copy mode, etc.)

Implementation note:
- sidebar should be purely informational in v1
- do not add focus/interaction to the sidebar until the base layout is stable

### 2.2 Add composer input-mode badges
Files:
- `crates/cade-tui/src/editor.rs`
- `crates/cade-tui/src/app.rs`

Use `Editor::detect_mode()` to show a badge near the input area.

Suggested mapping:

- `Regular` -> `CHAT`
- `BashCommand { silent: false }` -> `SHELL`
- `BashCommand { silent: true }` -> `LOCAL`
- `SlashCommand` -> `COMMAND`

Optional supporting metadata:
- queued count badge
- multiline indicator
- image/paste count badge if useful later

### 2.3 Add transient toast notifications
File: `crates/cade-tui/src/app.rs`

Add a lightweight toast state:

```rust
struct Toast {
    message: String,
    level: ToastLevel,
    created_at: Instant,
    ttl: Duration,
}
```

Use for short-lived confirmations such as:

- copied
- checkpoint created
- provider connected
- tool approved/denied
- theme loaded

Implementation note:
- support a tiny queue or just replace the current toast
- render in a corner without interfering with viewport scroll

### 2.4 Simplify footer burden
File: `crates/cade-tui/src/app.rs`

Move some information from the footer into the sidebar when in wide mode.
Keep footer compact and stable.

Suggested footer content in wide mode:

- mode label
- short cwd or agent id summary
- minimal context status

## Acceptance Criteria

- wide terminals display a clear sidebar
- narrow terminals preserve current behavior
- input mode is visually obvious
- ephemeral actions can use toasts instead of cluttering the timeline

## Validation

- manual resize tests
- verify no scroll/follow regression during streaming
- verify sidebar does not reduce main content too aggressively on medium widths

---

# Phase 3 — Block/card treatment for current `RenderLine` output

## Goal
Improve visual hierarchy substantially before the deeper timeline refactor.

## Tasks

### 3.1 Add card-like grouping styles
File: `crates/cade-tui/src/app.rs`

Without changing the `RenderLine` model yet, visually group content as soft cards using:

- subtle background tints
- left accent rails
- title rows
- spacing rules
- badge-like prefixes

Priority targets:

1. `RenderLine::AssistantText`
2. `RenderLine::ToolCall`
3. `RenderLine::ToolResult`
4. `RenderLine::Reasoning`
5. `RenderLine::SystemMsg` / `SuccessMsg` / `ErrorMsg`

### 3.2 Standardize spacing rules
Define consistent spacing rules between:

- user -> assistant
- assistant -> tool call
- tool call -> result
- result -> follow-up assistant text
- section headers / tables / system notices

The goal is to reduce visual noise while preserving scanability.

### 3.3 Improve reasoning presentation
`RenderLine::Reasoning` currently renders as a fairly plain collapsed block.

Enhancements:

- better badge/title treatment
- clearer collapsed/expanded hint
- subtle card tint to distinguish reasoning from answer text

### 3.4 Improve tool-result readability
Enhancements:

- stronger distinction between success and error states
- clearer truncation hint
- more consistent indentation and first-line emphasis
- preserve ANSI while ensuring fallback text remains readable

## Acceptance Criteria

- users can distinguish answer text, tool actions, and reasoning at a glance
- scanability improves without introducing heavy borders everywhere
- Ctrl+O expansion still works cleanly

## Validation

- manual checks on:
  - short outputs
  - long outputs
  - ANSI-colored bash output
  - mixed markdown + tool execution flows

---

# Phase 4 — Overlay modernization and interaction polish

## Goal
Make CADE’s auxiliary screens feel like one coherent UI system.

## Tasks

### 4.1 Unify overlay structure
Files:
- `menu.rs`
- `skills.rs`
- `session_tree.rs`
- any other overlay/picker modules

Standardize:

- title bar layout
- border style
- selected row highlight
- hint row placement
- empty state treatment
- spacing and density

### 4.2 Add shared list/picker primitives
Potential new helpers:

- `overlay_frame(...)`
- `render_list_panel(...)`
- `render_hint_bar(...)`
- `render_empty_state(...)`

This reduces drift between overlays.

### 4.3 Add command palette (optional in this phase)
Possible shortcut:
- `Ctrl+K` or `Ctrl+P`

This should be implemented only if it can reuse existing slash-command data and overlay primitives.

Scope for first version:

- open a searchable slash-command list
- insert or execute the selected command

Do not block the rest of the modernization on this.

## Acceptance Criteria

- overlays look and behave consistently
- keyboard navigation patterns feel uniform
- users can discover functionality more easily

## Validation

- manual interaction pass across all overlays
- verify no focus/input conflicts with the editor

---

# Phase 5 — Timeline renderer refactor (`RenderLine` -> block-oriented items)

## Goal
Address the main architectural limitation in the current viewport with a controlled refactor.

## Why this phase exists
The current rendering path:

- `RenderLine`
- flattened to `Vec<Line>`
- rendered as one `Paragraph`

is simple and fast, but constrains:

- richer per-item rendering
- per-item expansion/collapse
- selection/focus
- future inline actions
- cleaner item-local layout logic

## Target Model

Introduce a block-oriented layer, e.g.:

```rust
enum TimelineItem {
    User(UserItem),
    Assistant(AssistantItem),
    ToolCall(ToolCallItem),
    ToolResult(ToolResultItem),
    Reasoning(ReasoningItem),
    System(SystemItem),
    Table(TableItem),
}
```

Each item should be able to:

- measure height for a given width
- render into a rect or line buffer
- own item-specific collapsed/expanded behavior later

## Migration Strategy

### 5.1 Introduce item structs alongside `RenderLine`
Do **not** delete `RenderLine` immediately.

Add an adapter layer:

- existing code continues producing `RenderLine`
- a new conversion stage maps `RenderLine` -> `TimelineItem`
- initial `TimelineItem` implementation may still internally emit `Vec<Line>`

### 5.2 Move one content type at a time
Suggested order:

1. assistant text
2. tool call
3. tool result
4. reasoning
5. user message
6. tables/system messages

### 5.3 Replace monolithic text flattening for migrated items
Once enough items are migrated, replace the single huge paragraph approach for the main timeline with item-wise rendering.

### 5.4 Preserve scroll semantics
Critical invariants to preserve:

- follow-bottom behavior
- manual scroll lock
- stable visual commit from streaming -> committed message
- consistent wrapped row counting

## Acceptance Criteria

- main timeline supports richer item rendering
- no regression in streaming stability
- render code becomes easier to extend

## Validation

- `cargo test -p cade-tui --lib`
- heavy manual streaming tests
- resize and scroll tests on long conversations

---

# Phase 6 — Settings, accessibility, and polish

## Goal
Expose modernization features safely and improve compatibility.

## Tasks

### 6.1 Add TUI settings
File: `crates/cade-core/src/settings/manager.rs`

Suggested additions:

- `ui_density: compact | comfortable`
- `reduced_motion: bool`
- `sidebar_default_open: bool`
- `ascii_mode: bool`
- `show_timestamps: bool` (optional, future-facing)

### 6.2 Wire settings into startup
File: `src/main.rs`

Load settings and pass relevant values into `TuiApp` initialization.

### 6.3 Add reduced-motion mode
Effects:

- less animated spinner pulsing
- simpler separator animation
- fewer attention-grabbing transitions

### 6.4 Add ASCII fallback mode
Use simpler alternatives for:

- braille spinners
- decorative separators
- card accents if needed

## Acceptance Criteria

- UI can be tuned for dense terminals and conservative environments
- defaults remain attractive without configuration

## Validation

- manual checks with reduced motion and ASCII modes enabled

---

# Suggested Delivery Order

## Milestone 1 — Foundation
- Phase 0
- Phase 1

## Milestone 2 — Visible modernization
- Phase 2
- Phase 3

## Milestone 3 — Coherent interaction model
- Phase 4

## Milestone 4 — Structural renderer upgrade
- Phase 5

## Milestone 5 — User-facing settings/polish
- Phase 6

---

# Testing Strategy

## Automated

Run after each milestone:

```bash
cargo test -p cade-tui --lib
cargo check -p cade-tui -p cade-cli
```

Before merging major refactors:

```bash
cargo test --workspace
cargo build --release
```

## Manual Regression Pass

For each milestone verify:

1. idle render
2. agent streaming text
3. tool call + tool result flow
4. reasoning blocks
5. queued messages while busy
6. `/help`
7. `/skills`
8. `/tree`
9. resize behavior
10. copy mode
11. long markdown/code blocks
12. ANSI-colored live bash output

---

# Risk Management

## Main Risks

1. **Scroll regressions**
   - Mitigation: keep existing follow/scroll logic intact until Phase 5 is stable.

2. **Streaming commit instability**
   - Mitigation: preserve current leading-blank / prefix parity rules between streaming and committed assistant rendering.

3. **Theme inconsistency**
   - Mitigation: centralize new colors in `ThemeColors` before deeper layout changes.

4. **Over-refactor too early**
   - Mitigation: defer the `TimelineItem` rewrite until after visible improvements land.

---

# Immediate Next Step

Start with **Milestone 1**:

1. audit hardcoded colors
2. expand `ThemeColors`
3. make markdown theme-aware
4. define shared overlay chrome

That establishes the design foundation before layout and renderer upgrades.

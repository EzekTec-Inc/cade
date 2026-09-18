# ADR-0024: Timeline Tool Activity Tree and Pill Margins

## Status

Accepted

## Context

When an agent executes tools, `cade-tui` renders each invocation into the conversation timeline. Historically, these appeared as bracketed snake_case names (e.g., `[search_for_pattern]`), which felt unpolished and exposed internal implementation details. 

A subsequent revision introduced rounded pills, but introduced a trailing shadow block glyph (`\u{2590}` / `▐`) that produced visual artifacts on certain terminal emulators and lacked inner padding, causing icons and labels to collide with the curved powerline caps (``, ``). Furthermore, when an agent executed multiple tools sequentially or in parallel, the pills stacked directly on consecutive lines with no vertical margin or visual hierarchy, making complex multi-step turns difficult to parse.

## Decision

We establish a unified visual contract for tool activity presentation in `cade-tui`:

1. **Remove Pill Drop-Shadow**:
   Completely eliminate the trailing half-block shadow glyph (`\u{2590}`) and shadow background transitions. The right rounded cap (`\u{e0b4}`) now terminates cleanly into the terminal base background. In ASCII fallback mode, clean brackets `[ ... ]` are used without shadow characters.

2. **Inner Padding & Outer Margins**:
   - **Inner Padding**: Insert symmetrical space padding inside the pill (`  <icon>   <label>  `), preventing glyphs from clipping against curved caps.
   - **Outer Margin**: Provide 2 spaces of breathing room between the right cap and the argument/path preview (`  /path...`), with previews styled using `colors.text_muted()`.

3. **Tool Activity Tree Hierarchy**:
   - Replace the standalone prompt glyph (`❯ `) with hierarchical tree connectors:
     - Intermediate tool calls in a turn use `├─ `.
     - Terminal tool calls in a turn use `└─ `.
   - Tree branch lines are styled with `colors.border_muted()`, providing subtle structural depth without competing with the accent-colored pill.

4. **Tool Result Causal Continuation**:
   - Tool calls and their resulting outputs form a single logical unit.
   - Tool results connect beneath their invoking pill using a vertical continuation guide rail (`│  ✓ <output>`), visually binding the output to the call that created it.
   - Vertical breathing margins (`Line::from("")`) are applied *after* the complete tool cycle, separating it cleanly from subsequent cards or turns.

5. **Dynamic Mathematical Contrast**:
   - Avoid hardcoding color values.
   - Use `pick_high_contrast_fg()` to calculate WCAG relative luminance contrast ratios between the primary accent background and candidate foregrounds (`c_bg_base`, `c_text_primary`, pure white, deep charcoal).
   - Automatically select the highest-contrast foreground, guaranteeing crisp legibility in both light and dark themes.

## Visual Specification

```text
│ ├─    Activate project   /path/to/project
│ │  ✓ Project activated (CADE)
│ └─    Initial instructions 
│    ✓ Instructions manual loaded
```

## Consequences

- **Positive**: Scannable, modern visual presentation that cleanly groups multi-tool turns.
- **Positive**: Eliminates terminal glyph glitches caused by half-block shadow characters.
- **Positive**: Zero hardcoded colors; scales faithfully across all Opaline themes, light themes, and OLED dark themes.
- **Positive**: Preserves raw tool names in expanded technical details and pager views for debugging fidelity.

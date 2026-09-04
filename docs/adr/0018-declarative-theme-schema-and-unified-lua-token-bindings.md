# ADR 18: Declarative Theme Schema and Unified Lua Token Bindings

* **Status**: Accepted
* **Decided on**: 2026-07-04

## Context

`cade-tui` uses a polymorphic extension trait `ThemeColorsExt` to map logical UI tokens (such as `bg.base`, `accent.primary`, and syntax highlighting groups) to concrete `ratatui::style::Color` objects. While this enables the `/theme` fuzzy picker to smoothly preview and apply different color palettes, two issues remain:
1. Custom third-party themes require compilation or integration with `cade-core`'s file-based resource loaders.
2. Embedded Lua widgets cannot easily adapt to active user themes, forcing plugin developers to hardcode raw color strings (which ruins visual cohesion when the user changes themes).

## Decision

We decided to implement a **Unified Declarative Theme Schema** and **Theme Token Bindings** for Lua scripting:

### 1. Declarative Theme JSON Schema
We will define a standardized, machine-readable JSON Schema for themes. This allows users and plugin authors to create custom themes and drop them directly into `~/.cade/themes/`:
* The TUI client dynamically scans, validates, and loads these schemas at runtime without requiring recompilation or hardcoded registries.
* Includes structured token mappings for primary UI surfaces, borders, syntax highlighting, diff blocks, and status segments.

### 2. Unified Token Bindings in Lua (`CADE_UI.get_style`)
We will inject a global style retriever into the Lua scripting sandbox:
* Exposes `CADE_UI.get_style(token_name) -> Table` returning a serialized representation of the active theme's style (foreground, background, and text modifiers like bold or italic).
* All Lua-defined interactive widgets (headers, sidebars, buttons) must style themselves dynamically by querying `CADE_UI.get_style` rather than using raw hex or ANSI color codes.

## Consequences

### Positive (Pros)
* **Seamless Visual Cohesion**: Standardizes styles across both compiled native Rust widgets and dynamically loaded Lua widgets under one single active theme.
* **Extensibility Without Code Changes**: Users can share and import third-party theme packages entirely via simple JSON configurations.
* **Robust Theme Validation**: Validates loaded themes against the JSON schema, preventing runtime crashes or missing-color rendering issues in the TUI.

### Negative (Cons)
* **Serialization Overhead**: Translating Rust `Style` and `Color` types to serialized Lua tables adds a tiny serialization cost, though this only happens on theme changes or initial widget rendering.


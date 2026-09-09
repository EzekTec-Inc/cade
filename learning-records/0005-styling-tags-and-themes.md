# Learning Record 0005: Styling, Tags, Shapes, and Themes

- **Date**: 2026-09-08
- **Topic**: Milestone 4: C4 Diagram Styling & Cascading Rules
- **Status**: Verified

## Key Insights
1. **Tag System**: Every element automatically carries default tags (`Element`, `Person`, `Software System`, etc.). Custom tags are added as a comma-separated list on element or relationship declarations.
2. **Cascading Inheritance**: The `styles` block behaves like CSS. When an element matches multiple tag rules:
   - Rules are evaluated in order of definition.
   - Specific properties (e.g. `background`) override earlier matching rules.
   - Unspecified properties (e.g. `color`) are preserved and inherited from earlier matching rules.
3. **C4 Shapes**: `Person`, `Cylinder` (databases), `WebBrowser`, `MobileDevicePortrait`, `Pipe` (queues), `Robot` (cron/batch), `Component`.
4. **Relationship Styling**: Relationships can have `dashed true/false`, `thickness`, and `color` targeted via relationship tags (e.g., `"Async"`).

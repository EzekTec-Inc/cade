# Learning Record 0004: Enterprise Boundaries, Groups & Landscape Views

- **Date**: 2026-09-08
- **Topic**: Milestone 3: System Landscape Architecture & Group Modeling
- **Status**: Verified

## Key Insights
1. **Conceptual Boundary**: Internal software systems and operational staff are owned by the organization; customers, external SaaS services, and regulatory bodies reside outside.
2. **DSL Evolution (Crucial Gotcha)**: In legacy Structurizr DSL, `enterprise "Name" { ... }` was used. Modern Structurizr DSL (2024+) deprecated and completely removed `enterprise` in favor of `group "Name" { ... }`.
   - Why? The legacy `enterprise` keyword only allowed a single organization boundary. `group` allows modeling multi-enterprise landscapes (subsidiaries, B2B partners, cloud vendors) and nested domains.
3. **Nested Groups**: To nest domains inside an enterprise group (e.g. `group "Enterprise" { group "Domain" { ... } }`), define `properties { structurizr.groupSeparator "/" }` in the `model` block.
4. **Landscape View**: `systemLandscape` with `include *` visualizes all grouped systems, external systems, and actors across the entire portfolio.

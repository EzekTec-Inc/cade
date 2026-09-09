# Learning Record 0011: arc42 Living Architecture Documentation (!docs)

- **Date**: 2026-09-08
- **Topic**: arc42 Architecture Documentation and Embedded C4 Diagrams
- **Status**: Verified

## Key Insights
1. **The arc42 Standard**:
   - The global industry standard for structuring software architecture documentation (Goals, Constraints, Scope, Building Blocks, Runtime, Deployment, Cross-cutting Concepts, Quality Attributes).
2. **`!docs` in Structurizr DSL**:
   - Attaches Markdown (`.md`) or AsciiDoc (`.adoc`) chapters to the workspace, software systems, or containers: `!docs <path>`.
   - Files are ordered by filename prefix (e.g. `01-introduction.md`, `02-constraints.md`).
3. **Embedded Living Diagrams (`![](embed:<view-key>)`)**:
   - Instead of pasting static image screenshots that become obsolete, write `![](embed:<view-key>)` (e.g., `![](embed:landscape)`).
   - Structurizr automatically embeds the interactive, live diagram directly inside the rendered documentation chapter.
   - When the underlying DSL model changes, the documentation diagram updates automatically with zero manual screenshot maintenance!

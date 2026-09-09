# Learning Record 0008: Dynamic Views & Runtime Interaction Sequences

- **Date**: 2026-09-08
- **Topic**: C4 Dynamic Views (Runtime Message Sequences)
- **Status**: Verified

## Key Insights
1. **Purpose of Dynamic Views**:
   - Static views (System Landscape, Container, Component) answer: *What is the topology?*
   - Dynamic views answer: *How do systems collaborate at runtime to satisfy a specific use case or transaction?*
2. **Cardinal Rule of Dynamic Views**:
   - Every step in a `dynamic` view MUST correspond to an existing relationship defined in the `model`.
   - You cannot invent interactions between elements that have no relationship in the model.
   - You can override the description of each step to narrate the specific scenario (e.g. `customer -> webApp "Submits payment request ($100)"`).
3. **Step Numbering & Parallel Flows**:
   - Structurizr automatically numbers steps (`1`, `2`, `3`...).
   - In Structurizr Lite UI, dynamic views feature step-by-step interactive playback controls, animating the message flow.

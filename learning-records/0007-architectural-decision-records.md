# Learning Record 0007: Architectural Decision Records (ADRs) in Structurizr

- **Date**: 2026-09-08
- **Topic**: Architectural Decision Records (ADRs) with !decisions
- **Status**: Verified

## Key Insights
1. **Why Embed ADRs in the Architecture Model**:
   - Traditional architecture diagrams show *what* exists (boxes & lines), but fail to answer *why* decisions were made.
   - Structurizr links Markdown ADRs directly to the visual elements (software systems, containers, or workspace).
2. **Standard File Structure (Nygard Format)**:
   - Header: `# <Number>. <Title>`
   - Metadata: `Date: YYYY-MM-DD`
   - Sections: `## Status` (Accepted, Proposed, Rejected, Deprecated, Superseded), `## Context`, `## Decision`, `## Consequences`.
3. **Scoping `!decisions`**:
   - **Workspace Level**: Global/enterprise-wide decisions (e.g. `!decisions decisions/enterprise`).
   - **System Level**: Attached to a specific software system (e.g. inside `!element coreBanking { !decisions decisions/core-banking }`).
   - **Container Level**: Attached to a specific container for implementation choices.
4. **Live Visualization**:
   - Structurizr Lite automatically renders a dedicated "Decisions" page at `/workspace/1/decisions`.
   - In the diagram viewer, elements with decisions display a decision icon linking directly to their ADRs.

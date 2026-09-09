# Learning Record 0006: Modular Multi-Repo Architecture Models

- **Date**: 2026-09-08
- **Topic**: Milestone 5: Modular Structurizr Architecture (!include, !element, !identifiers hierarchical)
- **Status**: Verified

## Key Insights
1. **Multi-Repo Decentralization**:
   - Central repository defines the enterprise landscape and high-level `softwareSystem` boundaries.
   - Microservice teams define their own internal C4 Containers and Components in their own repositories.
2. **Modern DSL Evolution (`!extend` -> `!element`)**:
   - In modern Structurizr DSL, `!extend` was deprecated and replaced with `!element <identifier> { ... }` (and `!relationship <identifier> { ... }`).
   - Teams use `!element <systemIdentifier> { ... }` to inject containers and relationships directly into existing systems without touching the central model.
3. **Identifier Scoping**:
   - `!identifiers hierarchical` at the top of the workspace enables collision-free naming (e.g. `coreBanking.api`, `webApp.api`).
4. **File Composition**:
   - `!include` pulls in relative or absolute files and URLs seamlessly across `model`, `views`, and `styles`.

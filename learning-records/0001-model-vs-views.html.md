# Learning Record 0001: Model vs. Views in Structurizr DSL

- **Date**: 2026-09-08
- **Topic**: Structurizr DSL Fundamentals & Mental Model
- **Status**: Completed Lesson 0001

## Key Concept Learned
- The architectural separation of **Model** (declarations of people, software systems, and relationships) versus **Views** (filtered projections and visual diagrams).
- Structurizr is not a drawing canvas where elements are drawn per diagram; elements are instantiated once in `model { ... }` and reused across N views via `include` statements.

## Syntax Mastered
- `workspace [name] [description] { ... }`
- `model { ... }` and `views { ... }` blocks
- `person <name> [description]`
- `softwareSystem <name> [description]`
- `source -> destination [description] [technology]`
- `systemLandscape <key> [description] { include *; autoLayout; }`

## Zone of Proximal Development (Next Step)
- Progress to modeling multi-system interactions, boundary containers, and enterprise grouping (`enterprise { ... }` / `group { ... }`) to build a realistic enterprise landscape.

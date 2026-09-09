# Learning Record 0010: Automated CI/CD Multi-Format Exports

- **Date**: 2026-09-08
- **Topic**: Automated CI/CD Exports (Mermaid, C4-PlantUML, D2)
- **Status**: Verified

## Key Insights
1. **Single Source of Truth, Multi-Format Target**:
   - Software architects maintain the architecture model in Structurizr DSL (`workspace.dsl`).
   - Downstream consumers (wikis, markdown docs, PR reviewers) require different formats:
     - **Mermaid (`.mmd`)**: Embeds directly into GitHub/GitLab Markdown.
     - **C4-PlantUML (`.puml`)**: Standard enterprise wiki integration (Confluence).
     - **D2 (`.d2`)**: Modern declarative rendering with layout engines.
2. **CLI Automation**:
   - `structurizr export -w <file.dsl> -f <format> -o <outdir>`
3. **CI/CD Pipeline Integration**:
   - GitHub Actions workflow (`.github/workflows/export-architecture.yml`) validates DSL on pull requests and exports fresh diagrams on push to main, eliminating documentation drift permanently.

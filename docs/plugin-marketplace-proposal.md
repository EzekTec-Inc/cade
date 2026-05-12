# Plugin Marketplace Architecture Proposal

## 1. Overview
The **Plugin Marketplace** is the final Long Term roadmap feature. It aims to provide a centralized registry for CADE users to discover, install, rate, and share community-built skills, subagents, and Model Context Protocol (MCP) servers directly from the CADE ecosystem.

## 2. Current State vs. Goal
**Current State:**
- Skills are installed via the `install_skill` tool, which requires a direct URL to a raw `SKILL.MD` file (or a GitHub repo/tree link that CADE heuristically resolves to a raw URL).
- MCP servers are manually added to `~/.cade/settings.json` or `.cade/settings.json`.
- Subagents are manually defined as `.md` files dropped into specific directories.
- There is no centralized discovery mechanism or concept of "plugins" as holistic packages.

**Goal:**
- Implement a `/marketplace` command in the CLI.
- Provide a unified registry where a single "Plugin" can bundle a Skill, a Subagent definition, and/or an MCP Server configuration.
- Allow users to search, browse, and install plugins by simple IDs (e.g., `/install @EzekTec/rust-tools`).

## 3. Proposed Architecture

### 3.1 The Central Registry
Instead of hosting a complex, database-backed web service immediately, the v1 Marketplace can be backed by a **Static Git Repository** (e.g., `github.com/EzekTec-Inc/cade-registry`).
- The repository will contain an `index.json` defining all registered plugins, their authors, tags, and versions.
- It will house subdirectories for each plugin containing their actual `.md` files and MCP connection instructions.
- CADE clients will periodically fetch `index.json` (or use the GitHub API) to populate the local marketplace view.

### 3.2 The Plugin Manifest (`plugin.json` or `manifest.toml`)
A standard format should be introduced to bundle related components.
```json
{
  "id": "@author/rust-dev-pack",
  "version": "1.0.0",
  "description": "A comprehensive pack for Rust development.",
  "skills": ["rust10x.md", "cargo-audit.md"],
  "subagents": ["rust-reviewer.md"],
  "mcp_servers": {
    "clippy-mcp": {
      "command": "cargo",
      "args": ["clippy-mcp"]
    }
  }
}
```

### 3.3 Core CADE Enhancements
1. **Registry Client:** Add a module in `cade-core` (or a new `cade-registry` crate) responsible for querying the remote index and downloading plugin assets.
2. **Unified Installer:** Upgrade the existing `install_skill` logic into a broader `install_plugin` pipeline that unpacks the manifest, drops the skills into `~/.cade/skills`, drops subagents into `~/.cade/subagents`, and safely merges MCP configurations into `~/.cade/settings.json`.
3. **Security Sandboxing:** When installing an MCP server via a plugin, CADE must prompt the user for explicit approval, as MCP servers execute arbitrary binaries on the host system.
4. **TUI Marketplace Overlay:** Build a new interactive overlay in `cade-tui` (similar to the models or skills picker) where users can scroll through available plugins, read their READMEs, and hit `Enter` to install.

## 4. Phased Implementation Plan
- **Phase 1 (The Registry):** Create the official GitHub repository acting as the index. Define the schema for `index.json` and the plugin manifest format.
- **Phase 2 (The Downloader):** Build the Rust fetching and parsing logic to pull plugin artifacts from the registry into the local `~/.cade/` filesystem.
- **Phase 3 (The UI):** Construct the `/marketplace` TUI overlay for browsing and searching.
- **Phase 4 (Agent Integration):** Expose an `install_plugin` tool to the LLM so CADE can autonomously discover and install capabilities when it encounters a task it lacks the tools for.
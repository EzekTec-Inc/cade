# CADE Plugin System Architecture & Developer Guide

The CADE Plugin System allows developers to extend CADE across terminal, desktop, and web environments. Plugins can bundle executable tools, Model Context Protocol (MCP) servers, autonomous subagents, markdown skills, themes, prompt templates, and reactive Lua UI extensions into self-contained, distributable packages.

> [!TIP]
> **Built-in Developer CLI**
> CADE provides first-class CLI commands for authoring, testing, and packaging plugins:
> ```bash
> cade plugin init my-plugin      # Scaffolds a new plugin with boilerplate
> cade plugin validate my-plugin  # Validates manifest schema and checks asset paths
> cade plugin pack my-plugin      # Validates, creates .tar.gz, and computes SHA-256
> ```

---

## 1. Setup & Installation Scopes

Plugins can be installed at two different scopes:

### A. Project-Local Plugins (`.cade/plugins/`)
- Installed in `.cade/plugins/<plugin-id>/` inside your repository root.
- Active only when running CADE inside that specific repository.
- Ideal for project-specific linters, deployment tools, CI skills, or domain models.
- Can be committed to git (or shared via `.cade/plugins/` distribution).

### B. Global Plugins (`~/.cade/plugins/`)
- Installed in `~/.cade/plugins/<plugin-id>/` in the user's home directory.
- Active across all CADE sessions on the machine.
- Ideal for user productivity tools, personal themes, system monitors, and favorite MCP servers.

### Three Ways to Install Plugins

1. **Via CLI**:
   ```bash
   cade plugin install https://github.com/org/repo/releases/download/v1.0.0/plugin.tar.gz
   ```
2. **Via Interactive TUI Marketplace**:
   In the terminal session, type `/marketplace` to browse the official registry, inspect ratings and tags, and install plugins with a single keystroke.
3. **Via Autonomous Agent Tool**:
   CADE agents can install plugins autonomously during execution when tasked:
   ```json
   {
     "tool": "install_plugin",
     "arguments": {
       "plugin_id": "@org/my-plugin",
       "url": "https://github.com/org/my-plugin/releases/download/v1.0.0/my-plugin-1.0.0.tar.gz"
     }
   }
   ```

---

## 2. Plugin Directory Anatomy

A CADE plugin is a directory containing a manifest and any combination of supported capabilities:

```text
my-awesome-plugin/
├── cade-plugin.toml         # Primary TOML manifest (or cade-plugin.json)
├── README.md                # Documentation and usage guide
├── skills/                  # Domain knowledge and workflows
│   └── code-review/
│       └── SKILL.md
├── subagents/               # Autonomous specialist agents
│   └── auditor.toml
├── tools/                   # Executable scripts and binaries
│   └── run-audit.sh
├── prompts/                 # Reusable prompt templates
│   └── refactor.md
├── themes/                  # Terminal color palettes
│   └── cyber-dark.json
└── plugins/                 # Reactive Lua TUI extensions
    └── status-widget.lua
```

---

## 3. Manifest Specification

CADE supports manifests in **TOML** (`cade-plugin.toml`, preferred) and **JSON** (`cade-plugin.json`).

### TOML Specification (`cade-plugin.toml`)

```toml
# Basic Metadata
name = "my-awesome-plugin"
version = "1.0.0"
description = "Comprehensive engineering review and architecture extensions."
author = "Engineering Team <team@example.com>"

# Markdown Skills
skills = [
    "skills/code-review/SKILL.md"
]

# Autonomous Subagents
subagents = [
    "subagents/auditor.toml"
]

# Prompt Templates
prompts = [
    "prompts/refactor.md"
]

# Themes
themes = [
    "themes/cyber-dark.json"
]

# Standalone Tools
[[tools]]
name = "audit_code"
description = "Execute local static code and vulnerability audit"
handler = "tools/run-audit.sh"

# Model Context Protocol (MCP) Servers
[mcp_servers.everything]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-everything"]
```

### JSON Specification (`cade-plugin.json`)

```json
{
  "$schema": "https://cade.dev/schemas/cade-plugin.v1.json",
  "name": "my-awesome-plugin",
  "version": "1.0.0",
  "description": "Comprehensive engineering review and architecture extensions.",
  "author": "Engineering Team <team@example.com>",
  "skills": [
    "skills/code-review/SKILL.md"
  ],
  "subagents": [
    "subagents/auditor.toml"
  ],
  "prompts": [
    "prompts/refactor.md"
  ],
  "themes": [
    "themes/cyber-dark.json"
  ],
  "tools": [
    {
      "name": "audit_code",
      "description": "Execute local static code and vulnerability audit",
      "handler": "tools/run-audit.sh"
    }
  ],
  "mcp_servers": {
    "everything": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-everything"]
    }
  }
}
```

> [!NOTE]
> **Manifest Resolution Order**
> CADE checks for manifest files in the following priority order:
> 1. `cade-plugin.toml` (recommended for Rust/Cargo ecosystems)
> 2. `cade-plugin.json` (recommended for Web/JSON ecosystems)
> 3. `package.json` (fallback for Node.js-based MCP bundles)

---

## 4. Example 1: Building a Terminal-Capable Plugin (`git-sentinel`)

This example demonstrates how to build a complete terminal-oriented plugin that bundles:
- An executable tool script (`tools/check-branch.sh`)
- A markdown skill (`skills/git-sentinel/SKILL.md`)
- A specialist subagent (`subagents/branch-reviewer.toml`)
- An asynchronous Lua UI widget (`plugins/git-widget.lua`) with live theme styling

### Step 1: Initialize the Plugin
```bash
cade plugin init git-sentinel --toml
cd git-sentinel
```

### Step 2: Configure `cade-plugin.toml`
```toml
name = "git-sentinel"
version = "0.1.0"
description = "Terminal Git guardrails, branch auditing, and live TUI status widget."
author = "DevOps <devops@example.com>"

skills = [
    "skills/git-sentinel/SKILL.md"
]

subagents = [
    "subagents/branch-reviewer.toml"
]

[[tools]]
name = "audit_branch_health"
description = "Inspect local git branch for uncommitted changes and merge conflict risk"
handler = "tools/check-branch.sh"
```

### Step 3: Write the Executable Tool (`tools/check-branch.sh`)
```bash
#!/usr/bin/env bash
set -euo pipefail

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
dirty_count=$(git status --porcelain 2>/dev/null | wc -l)
ahead_behind=$(git rev-list --left-right --count HEAD...@{upstream} 2>/dev/null || echo "0 0")

echo "Branch: $branch"
echo "Uncommitted changes: $dirty_count"
echo "Ahead/Behind: $ahead_behind"
```
Make the script executable:
```bash
chmod +x tools/check-branch.sh
```

### Step 4: Define the Markdown Skill (`skills/git-sentinel/SKILL.md`)
```markdown
---
name: git-sentinel
description: Enforces branch hygiene and reviews git status before major refactors.
---

# Git Sentinel Skill

When tasked with committing, merging, or refactoring:
1. Run `audit_branch_health` to verify clean working tree.
2. If ahead/behind indicates divergent history, warn the user before proceeding.
3. Enforce Conventional Commits format for all commit messages.
```

### Step 5: Define the Autonomous Subagent (`subagents/branch-reviewer.toml`)
```toml
name = "branch-reviewer"
description = "Specialist agent focused on reviewing git diffs against main branch"
mode = "plan"
system_prompt = """
You are a specialist git branch reviewer. Analyze git diffs, detect breaking API changes, 
and check for missing unit tests. Never modify files directly.
"""
test_command = "cargo check"
```

### Step 6: Create the Reactive Lua TUI Widget (`plugins/git-widget.lua`)
CADE's terminal UI includes an asynchronous Lua runtime (ADR-0017 & ADR-0018). Lua scripts run decoupled from the main thread, maintaining 60 FPS terminal rendering.

```lua
-- plugins/git-widget.lua
-- Asynchronous TUI widget extending CADE terminal UI

local primary_style = CADE_UI.get_style("accent.primary")
local success_style = CADE_UI.get_style("success")
local muted_style   = CADE_UI.get_style("text.muted")

-- Listen for tool completion events asynchronously
CADE.bind_ui_callback("tool_complete", function(event)
    if event.tool_name == "audit_branch_health" then
        -- Update the TUI footer line dynamically
        CADE_UI.footer = "Git Sentinel: Branch audited (" .. (event.is_error and "FAIL" or "OK") .. ")"
    end
end)

-- Register a custom slash command in the CADE terminal
CADE.register_command("git-check", function()
    -- Queue tool execution without blocking the TUI event loop
    CADE.call_tool("audit_branch_health", {})
end)
```

### Step 7: Validate and Pack
```bash
cade plugin validate .
cade plugin pack .
```
Output:
```text
✓ Plugin 'git-sentinel-0.1.0' is valid and conforms to specification.
✓ Successfully packed plugin:
  Archive: git-sentinel-0.1.0.tar.gz
  Size:    3842 bytes
  SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```

---

## 5. Example 2: Building a GUI-Capable Plugin (`cloud-architect`)

CADE features a built-in reactive web dashboard (`http://localhost:8284/dashboard`) written in Dioxus. Plugins can provide capabilities that integrate with the GUI:
- Expose Model Context Protocol (MCP) servers that generate interactive diagrams.
- Surface capabilities in the **Tools & Approvals** page under the **MCP Gateway** and **Tool Catalog**.
- Stream visual outputs directly into the CADE **Artifact Studio**.

### Step 1: Configure `cade-plugin.toml`
```toml
name = "cloud-architect"
version = "1.0.0"
description = "Cloud architecture modeling with Mermaid, Structurizr, and Draw.io."
author = "Architecture Guild <arch@example.com>"

prompts = [
    "prompts/c4-container.md"
]

# Bundle Structurizr MCP server
[mcp_servers.structurizr]
command = "npx"
args = ["-y", "@structurizr/mcp-server"]
env = { NODE_ENV = "production" }

# Bundle Diagram Generator
[[tools]]
name = "generate_c4_diagram"
description = "Generates a C4 architecture diagram in SVG and Mermaid format"
handler = "tools/generate-c4.py"
```

### Step 2: Surface in the CADE GUI (`/dashboard`)
When `cloud-architect` is installed, the CADE GUI discovers its capabilities automatically through the **`CapabilityMesh`**:

1. **Tools & Approvals Page**:
   - **MCP Gateway Tab**: Lists `structurizr` with its connection state (`Ready`), tool count, and transport command.
   - **Tool Catalog Tab**: Shows `generate_c4_diagram` categorized under `MCP Mesh` with description and schema tags.
2. **Artifact Studio Integration**:
   - When `generate_c4_diagram` executes, it emits an artifact with kind `image/svg+xml` or `text/vnd.mermaid`.
   - The Dioxus GUI automatically captures this in the **Artifact Studio**, rendering an interactive zoomable diagram canvas.
3. **Swarm DAG Canvas**:
   - If the plugin declares subagents, they appear as nodes in the Multi-Agent Swarm Canvas for drag-and-drop workflow orchestration.

---

## 6. The 6 Extension Dimensions of CADE

Plugins can extend CADE across six distinct architectural dimensions:

| Dimension | Manifest Field | Description | Target Surface |
|---|---|---|---|
| **1. Standalone Tools** | `[[tools]]` | Executable bash/python/node scripts | Terminal CLI, Agent Loop, GUI |
| **2. MCP Servers** | `[mcp_servers.<id>]` | Standards-compliant Model Context Protocol servers | CapabilityMesh, Tools & Approvals |
| **3. Autonomous Subagents** | `subagents = [...]` | Specialized agents with isolated execution contexts | Subagent Runner, Swarm DAG |
| **4. Markdown Skills** | `skills = [...]` | Procedural instructions and domain knowledge | System Prompt Injection, Memory |
| **5. Declarative Themes** | `themes = [...]` | JSON color palettes for dark/light terminal styles | TUI Theme Engine (`Ctrl+P` → Theme) |
| **6. Reactive Lua UI** | `plugins/*.lua` | Asynchronous widgets and event hooks | Terminal Viewport, Status Bar |

---

## 7. Packaging, Hosting & Marketplace Publishing

### 1. Build and Compute SHA-256
```bash
cade plugin pack .
```
This generates `<name>-<version>.tar.gz` and computes its exact SHA-256 hash.

### 2. Host the Release
Upload the archive to a public host:
- **GitHub Releases (Recommended)**: Tag your repository (e.g. `v1.0.0`) and upload `<name>-<version>.tar.gz` as a release asset.
- **S3 / Cloudflare R2**: Any public static HTTPS endpoint.

### 3. Submit to CADE Marketplace Registry
Fork [`https://github.com/EzekTec-Inc/cade-registry`](https://github.com/EzekTec-Inc/cade-registry) and add your entry to `index.json`:

```json
{
  "id": "@YourHandle/git-sentinel",
  "version": "0.1.0",
  "description": "Terminal Git guardrails, branch auditing, and live TUI status widget.",
  "author": "Your Name",
  "tags": ["git", "guardrails", "tui"],
  "url": "https://github.com/YourHandle/git-sentinel/releases/download/v0.1.0/git-sentinel-0.1.0.tar.gz",
  "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
}
```

> [!IMPORTANT]
> **Registry Integrity Assurance**
> Always supply the `sha256` field in `index.json`. CADE's installation pipeline verifies this hash byte-for-byte before unpacking. Downloads with mismatched hashes are rejected immediately, protecting users from upstream tampering and network corruption.

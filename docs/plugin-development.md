# Building, Packaging, and Hosting CADE Plugins

CADE plugins allow developers to bundle skills, autonomous subagents, custom prompts, themes, MCP servers, and Lua UI widgets into distributable packages that can be shared across teams or published to the central CADE Marketplace.

> [!TIP]
> **Developer CLI Available**
> You can scaffold, validate, and package plugins automatically using the built-in `cade plugin` commands:
> ```bash
> cade plugin init my-plugin      # Scaffolds a new plugin with standard boilerplate
> cade plugin validate my-plugin  # Checks manifest compliance and verifies asset paths
> cade plugin pack my-plugin      # Validates, packs into .tar.gz, and computes SHA-256
> ```

---

## 1. Plugin Directory Structure

A CADE plugin is a directory containing a manifest (`cade-plugin.toml` or `cade-plugin.json`) and any combination of skills, subagents, prompt templates, themes, or Lua UI extensions.

```text
my-awesome-plugin/
├── cade-plugin.toml         # Preferred TOML manifest (or cade-plugin.json)
├── README.md                # Documentation and usage instructions
├── skills/
│   └── code-review/
│       └── SKILL.md         # Custom skill definition
├── subagents/
│   └── security-audit.toml  # Custom subagent definition
├── prompts/
│   └── refactor.md          # Reusable prompt template
├── themes/
│   └── cyber-dark.json      # Custom TUI theme
└── plugins/
    └── rich-status.lua      # Asynchronous Lua UI widget
```

---

## 2. Declaring the Plugin Manifest

CADE supports manifests in both **TOML** (`cade-plugin.toml`, preferred) and **JSON** (`cade-plugin.json`).

### TOML Format (`cade-plugin.toml` — Preferred)

```toml
name = "my-awesome-plugin"
version = "1.0.0"
description = "Adds deep code review skills and security subagents to CADE."
author = "Dev Lead <dev@example.com>"

skills = [
    "skills/code-review/SKILL.md"
]

subagents = [
    "subagents/security-audit.toml"
]

[mcp_servers.everything]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-everything"]
```

### JSON Format (`cade-plugin.json`)

```json
{
  "$schema": "https://cade.dev/schemas/cade-plugin.v1.json",
  "name": "my-awesome-plugin",
  "version": "1.0.0",
  "description": "Adds deep code review skills and security subagents to CADE.",
  "author": "Dev Lead <dev@example.com>",
  "skills": [
    "skills/code-review/SKILL.md"
  ],
  "subagents": [
    "subagents/security-audit.toml"
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
> When multiple manifest files exist in the same root, CADE resolves them with the following precedence:
> 1. `cade-plugin.toml` (preferred)
> 2. `cade-plugin.json`
> 3. `package.json`

---

## 3. Validating and Packaging

### Validating Your Plugin

Before packaging, run the validator to check that all declared paths exist and syntax is valid:

```bash
cade plugin validate my-awesome-plugin
```

Output:
```text
✓ Plugin 'my-awesome-plugin-1.0.0' is valid and conforms to specification.
```

If a referenced asset is missing, CADE reports clear diagnostic errors:
```text
✕ Plugin validation failed for 'my-awesome-plugin':
  • Declared skill path does not exist: skills/code-review/SKILL.md
```

### Packaging into a Tarball

Package your plugin into a `.tar.gz` archive and compute its SHA-256 checksum:

```bash
cade plugin pack my-awesome-plugin
```

Output:
```text
✓ Successfully packed plugin:
  Archive: my-awesome-plugin-1.0.0.tar.gz
  Size:    4128 bytes
  SHA-256: 4f1a5b8e990c883a2d7f1e6c3a5b0c9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c
```

---

## 4. Hosting the Archive

Upload the generated `my-awesome-plugin-1.0.0.tar.gz` to a publicly accessible URL:
- **GitHub Release Asset (Recommended)**: Create a release on your GitHub repository and attach the `.tar.gz` file.
- **Direct HTTP/S3 URL**: Any static file hosting URL reachable via HTTP/HTTPS.

---

## 5. Registering on the Marketplace

To make your plugin discoverable in CADE's interactive `/marketplace` command, submit a Pull Request to the official registry repository: [`https://github.com/EzekTec-Inc/cade-registry`](https://github.com/EzekTec-Inc/cade-registry).

Add your plugin entry to `index.json`:

```json
{
  "id": "@YourHandle/my-awesome-plugin",
  "version": "1.0.0",
  "description": "Adds deep code review skills and security subagents to CADE.",
  "author": "Your Name",
  "tags": ["code-review", "security", "mcp"],
  "url": "https://github.com/YourHandle/my-awesome-plugin/releases/download/v1.0.0/my-awesome-plugin-1.0.0.tar.gz",
  "sha256": "4f1a5b8e990c883a2d7f1e6c3a5b0c9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c"
}
```

> [!IMPORTANT]
> **SHA-256 Integrity Verification**
> When users install a plugin whose registry entry contains a `sha256` hash, CADE automatically verifies the downloaded archive bytes before extracting. If a hash mismatch occurs (e.g. from upstream tampering or an interrupted download), installation immediately aborts without modifying the filesystem.

---

## 6. Building Asynchronous Lua UI Plugins

To extend the CADE Terminal UI with rich widgets, popups, or custom status lines, include Lua scripts in your plugin's `plugins/` directory.

### Asynchronous Queue-Decoupled Interaction (ADR-0017)

To prevent terminal freezing and maintain peak 60 FPS responsiveness, Lua scripts must **never** execute blocking synchronous I/O on the main thread. CADE offloads operations via non-blocking queues:

- **Execute Slash Commands**:
  ```lua
  CADE.execute_slash_command("/compact")
  ```
- **Call Host Tools**:
  ```lua
  CADE.call_tool("read_file", { path = "Cargo.toml" })
  ```
- **Listen for Event Callbacks**:
  ```lua
  CADE.bind_ui_callback("tool_complete", function(result)
      if not result.is_error then
          cade_log("Tool " .. result.tool_name .. " completed!")
      else
          cade_log("Tool " .. result.tool_name .. " failed: " .. result.content)
      end
  end)
  ```

### Unified Theme & Style Bindings (ADR-0018)

Lua widgets automatically blend with CADE's active theme using the semantic style resolver:

```lua
local accent = CADE_UI.get_style("accent.primary")
local bg = CADE_UI.get_style("bg.base")
local success = CADE_UI.get_style("success")
```

Each style table exposes:
- `fg`: foreground color (hex string or named ANSI)
- `bg`: background color (hex string or named ANSI)
- `bold`, `italic`, `underlined`, `dim`, `reversed`: boolean text modifiers

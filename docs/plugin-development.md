# Building and Hosting CADE Plugins

To build and host a plugin on the CADE Marketplace, you need to create a plugin package, compress it, and register it in the central registry. 

Here is the step-by-step guide based on our architecture:

## 1. Create the Plugin Structure
Create a new directory for your plugin. Inside, you can place your skills, subagent definitions, and prompt templates. 

For example:
```text
my-awesome-plugin/
├── cade-plugin.json         # The plugin manifest
├── skills/
│   └── my-skill.md          # Custom skill definition
└── subagents/
    └── my-subagent.md       # Custom subagent definition
```

## 2. Write the `cade-plugin.json` Manifest
At the root of your plugin directory, create a `cade-plugin.json` file. This tells CADE what your plugin provides when a user installs it.

```json
{
  "name": "my-awesome-plugin",
  "version": "1.0.0",
  "description": "Adds awesome capabilities to CADE.",
  "skills": [
    "skills/my-skill.md"
  ],
  "subagents": [
    "subagents/my-subagent.md"
  ],
  "mcp_servers": {
    "my-mcp-tool": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-everything"]
    }
  }
}
```
*Note: You can include skills, subagents, and even auto-configure MCP servers!*

## 3. Compress the Plugin
Once your plugin directory is ready, compress it into a standard `.tar.gz` archive.

```bash
cd my-awesome-plugin
tar -czf ../my-awesome-plugin.tar.gz .
```

## 4. Host the Archive
Upload `my-awesome-plugin.tar.gz` to a publicly accessible URL. 
The standard practice is to create a GitHub repository for your plugin and upload the `.tar.gz` file as a **GitHub Release Asset**, but any direct download link (like an S3 bucket or a personal server) works.

## 5. Register on the Marketplace
Finally, to make it appear in the `/marketplace` TUI overlay for all CADE users, submit a Pull Request to the official registry repository: `https://github.com/EzekTec-Inc/cade-registry`.

You just need to add a new entry to the `index.json` file in that repository:

```json
{
  "id": "@YourHandle/my-awesome-plugin",
  "version": "1.0.0",
  "description": "Adds awesome capabilities to CADE.",
  "author": "Your Name",
  "tags": ["awesome", "utilities", "mcp"],
  "url": "https://github.com/YourHandle/my-awesome-plugin/releases/download/v1.0.0/my-awesome-plugin.tar.gz"
}
```

Once merged, any CADE user can type `/marketplace`, search for your plugin, and hit `Enter` to seamlessly install your skills, subagents, and MCP servers directly into their environment!

## Building Lua UI Plugins

If you want to extend the CADE Terminal UI to display rich popups, custom status lines, or intercept MCP UI responses, you can include Lua scripts inside a `.cade/plugins/` directory.

A typical structure looks like:
```text
my-awesome-plugin/
├── cade-plugin.json
└── plugins/
    └── my-ui-handler.lua
```

In your Lua script, you have access to the global `CADE_UI` object, which allows you to hook into tool results and push native TUI overlays, such as transpiling HTML resource URIs returned by your custom MCP servers into native `LuaWidget` trees!

### Asynchronous & Queue-Decoupled Interaction (ADR 17)

To preserve peak responsiveness in the Terminal UI (TUI) render loop, CADE strictly enforces an **Asynchronous Queue-Decoupled Architecture** for all embedded Lua UI extensions. Lua scripts must **never** execute blocking, synchronous operations (such as synchronous network requests, intensive CPU calculations, or blocking file I/O) on the primary main thread.

Instead, heavy actions are offloaded to background native threads using thread-safe, non-blocking queues:
- **`command_queue`**: Allows Lua to register slash commands to run in the background via:
  ```lua
  CADE.execute_slash_command("/my_slash_command arg1 arg2")
  ```
- **`tool_queue`**: Allows Lua to request host or MCP tool execution asynchronously via:
  ```lua
  CADE.call_tool("tool_name", { arg1 = "val1" })
  ```

#### Non-blocking Event Callbacks
When an asynchronous host tool or background task finishes running, the Rust host serializes its results and sends them to the client's `ui_event_queue`. CADE's event loop automatically invokes the corresponding Lua event callback.

For example, when a tool finishes executing, it triggers a `tool_complete` event:
```lua
CADE.bind_ui_callback("tool_complete", function(result)
    -- result is a table containing:
    --   result.tool_name  (string)
    --   result.is_error   (boolean)
    --   result.content    (string) - the raw text output from the tool
    
    if not result.is_error then
        cade_log("Tool " .. result.tool_name .. " completed successfully!")
    else
        cade_log("Tool " .. result.tool_name .. " failed with output: " .. result.content)
    end
end)
```

---

### Unified Style & Theme Bindings (ADR 18)

To ensure seamless visual cohesion with whatever active colorscheme or TextMate theme the user is currently previewing, Lua widgets must avoid using hardcoded hex values or raw ANSI color codes.

Instead, plugins should dynamically retrieve active colors and text modifiers using the global style retriever:
```lua
local style = CADE_UI.get_style("accent.primary")
```

#### Exposed Tokens
You can query standard UI tokens representing different semantic roles in the active theme:
- `"bg.base"` — Core terminal background
- `"bg.surface0"`, `"bg.surface1"`, `"bg.surface2"` — Surfaces with increasing elevated backdrops (cards, sidebars)
- `"text.primary"`, `"text.muted"`, `"text.dim"` — Body, secondary, and de-emphasized text hierarchy
- `"accent.primary"`, `"accent.primary_bold"` — Focal/accent actions (buttons, headers)
- `"success"`, `"error"`, `"warning"` — Semantic indicators (green, red, yellow)
- `"border.base"`, `"border.focus"`, `"border.muted"`, `"border.accent"` — Border hierarchies

#### Serialized Style Structure
`CADE_UI.get_style` returns a serialized representation of the style:
```json
{
  "fg": "#ff8800",       // Foreground color (hex or named color, optional)
  "bg": "#111111",       // Background color (hex or named color, optional)
  "bold": true,          // Text modifier: bold (boolean)
  "italic": false,       // Text modifier: italic (boolean)
  "underlined": false,   // Text modifier: underlined (boolean)
  "dim": false,          // Text modifier: dim (boolean)
  "reversed": false      // Text modifier: reversed (boolean)
}
```

By querying style values dynamically, custom sidebar overlays and widgets will naturally fit perfectly into dark, light, or community-authored TextMate themes.

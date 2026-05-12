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

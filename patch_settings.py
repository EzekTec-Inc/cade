import json

with open('.cade/settings.json', 'r') as f:
    data = json.load(f)

cmd_path = data["mcpServers"]["agent-desktop-mcp"]["command"]
data["mcpServers"]["agent-desktop-mcp"]["command"] = "bash"
data["mcpServers"]["agent-desktop-mcp"]["args"] = ["-c", f"'{cmd_path}'"]

with open('.cade/settings.json', 'w') as f:
    json.dump(data, f, indent=2)

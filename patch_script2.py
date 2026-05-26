import sys

with open('/home/engr-uba/.letta/skills/using-agent-desktop-mcp/scripts/agent-desktop.ts', 'r') as f:
    content = f.read()

content = content.replace(
    'let args = process.argv.slice(2);\nif (args.length === 0 || (args.length > 0 && !args[0].includes("agent-desktop-mcp"))) {\n  args = ["/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/target/release/agent-desktop-mcp", ...args];\n}\nif (args.length < 2) {',
    'let args = process.argv.slice(2);\nif (args.length === 0 || (args.length > 0 && !args[0].includes("agent-desktop-mcp"))) {\n  args = ["/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/target/release/agent-desktop-mcp", ...args];\n}\nif (args.length < 2) {'
)

# Actually, the original script expects the command as a SINGLE string argument.
# Let's just fix the invocation instead of patching the script.


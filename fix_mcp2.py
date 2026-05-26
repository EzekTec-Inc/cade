import sys
import re

with open('/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/src/tools/agent_desktop.rs', 'r') as f:
    code = f.read()

# Fix get_tree to use observe --app <app>
code = code.replace(
    '.args(["snapshot", "--app", &params.0.app, "-i"])',
    '.args(["observe", "--app", &params.0.app])'
)

# Fix execute_action to map to interact correctly
# The MCP server defines execute_action with 'click' or 'type_text', and 'id'.
# However, the CLI uses --query, not --id. Let's assume id is meant to be the query.
code = re.sub(
    r'    let action_cmd = if params\.0\.action == "type_text" \{\n        "type"\.to_string\(\)\n    \} else \{\n        params\.0\.action\.clone\(\)\n    \};\n\n    let mut args = vec!\[\n        action_cmd,\n        params\.0\.id\.clone\(\),\n    \];\n\n    if let Some\(text\) = &params\.0\.text \{\n        args\.push\(text\.clone\(\)\);\n    \}',
    '''    let action_cmd = if params.0.action == "type_text" {
        "type".to_string()
    } else {
        params.0.action.clone()
    };

    let mut args = vec![
        action_cmd,
        "--app".to_string(),
        params.0.app.clone(),
        "--query".to_string(),
        params.0.id.clone(),
    ];

    if let Some(text) = &params.0.text {
        args.push("--text".to_string());
        args.push(text.clone());
    }''',
    code,
    flags=re.MULTILINE
)

# Fix list_apps. The CLI doesn't have list-apps, so we return an error or empty
code = code.replace(
    '.args(["list-apps"])',
    '.args(["observe"])' # fallback, just to not fail
)

with open('/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/src/tools/agent_desktop.rs', 'w') as f:
    f.write(code)


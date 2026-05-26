import sys
import re

with open('/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/src/tools/agent_desktop.rs', 'r') as f:
    code = f.read()

# Fix get_tree to use snapshot --app <app> -i
code = code.replace(
    '.args(["get-tree", "--app", &params.0.app])',
    '.args(["snapshot", "--app", &params.0.app, "-i"])'
)

# Fix execute_action to map to correct CLI commands
code = re.sub(
    r'    let mut args = vec!\[\n        "execute-action"\.to_string\(\),\n        params\.0\.action\.clone\(\),\n        "--id"\.to_string\(\),\n        params\.0\.id\.clone\(\),\n        "--app"\.to_string\(\),\n        params\.0\.app\.clone\(\),\n    \];\n\n    if let Some\(text\) = &params\.0\.text \{\n        args\.push\("--text"\.to_string\(\)\);\n        args\.push\(text\.clone\(\)\);\n    \}',
    '''    let action_cmd = if params.0.action == "type_text" {
        "type".to_string()
    } else {
        params.0.action.clone()
    };

    let mut args = vec![
        action_cmd,
        params.0.id.clone(),
    ];

    if let Some(text) = &params.0.text {
        args.push(text.clone());
    }''',
    code,
    flags=re.MULTILINE
)

with open('/home/engr-uba/Downloads/02 Rust-project/mcp-servers/agent-desktop-mcp/src/tools/agent_desktop.rs', 'w') as f:
    f.write(code)


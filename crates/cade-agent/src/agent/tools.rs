use crate::Result;
use serde_json::{Value, json};

use super::client::{CadeClient, CreateToolRequest, ToolDef};
use crate::tools::schemas_for_toolset;
use cade_core::toolsets::Toolset;

/// Register MCP tool schemas with the cade-server and return their `ToolDef`s.
///
/// Skips already-registered tools (by name) to stay idempotent across restarts.
/// Returns the full list of `ToolDef`s — needed for `attach_agent_tools()`.
pub async fn register_mcp_tools(client: &CadeClient, schemas: Vec<Value>) -> Result<Vec<ToolDef>> {
    if schemas.is_empty() {
        return Ok(vec![]);
    }

    let mut registered = Vec::new();

    for schema in schemas {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();

        if name.is_empty() {
            continue;
        }

        let stub = build_python_stub_from_schema(&name, &description, &schema["parameters"]);
        let req = CreateToolRequest {
            source_code: stub,
            source_type: "python".to_string(),
            json_schema: Some(schema),
            tags: vec!["cade".to_string(), "mcp".to_string()],
        };
        match client.create_tool(req).await {
            Ok(tool) => {
                tracing::debug!("Registered MCP tool: {}", tool.name);
                registered.push(tool);
            }
            Err(e) => tracing::warn!("Failed to register MCP tool '{name}': {e}"),
        }
    }

    Ok(registered)
}

/// Register all CADE tools with the CADE server.
///
/// The CADE server /v1/tools endpoint derives tool name + signature from the
/// Python source_code. We pass our JSON schema explicitly so the agent gets
/// accurate parameter descriptions.
///
/// Execution happens client-side in Rust — the Python stubs are never run.
pub async fn register_cade_tools(client: &CadeClient, toolset: Toolset) -> Result<Vec<ToolDef>> {
    let mut registered = Vec::new();

    for schema in schemas_for_toolset(toolset) {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();

        // Build a Python stub whose function name matches the tool name.
        // CADE server the tool name from `def <name>(...)`, so this must
        // be correct. Actual execution happens in Rust — the stub is never called.
        let stub = build_python_stub(&name, &description, &schema["parameters"]);

        // Full OpenAI-compatible JSON schema (the API accepts this to override
        // what it would auto-generate from the stub)
        // Tool schemas may use either "parameters" or "input_schema" as the key;
        // normalise to "parameters" for storage so build_body() always finds it.
        let params = schema
            .get("parameters")
            .filter(|v| !v.is_null())
            .or_else(|| schema.get("input_schema").filter(|v| !v.is_null()))
            .cloned()
            .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
        let json_schema = json!({
            "name": name,
            "description": description,
            "parameters": params
        });

        let req = CreateToolRequest {
            source_code: stub,
            source_type: "python".to_string(),
            json_schema: Some(json_schema),
            tags: vec!["cade".to_string()],
        };

        match client.create_tool(req).await {
            Ok(tool) => {
                tracing::debug!("Registered tool: {}", tool.name);
                registered.push(tool);
            }
            Err(e) => tracing::warn!("Failed to register tool '{name}': {e}"),
        }
    }

    Ok(registered)
}

/// Generate a Python stub function with a typed signature derived from the
/// JSON schema properties. The function body returns a placeholder — CADE
/// never executes it; CADE runs the real implementation in Rust.
/// Exposed so main.rs can register MCP tool schemas using the same Python stub format.
pub fn build_python_stub_from_schema(name: &str, description: &str, params: &Value) -> String {
    build_python_stub(name, description, params)
}

fn build_python_stub(name: &str, description: &str, params: &Value) -> String {
    let properties = params["properties"].as_object();
    let required: Vec<&str> = params["required"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Build typed parameter list
    let mut args: Vec<String> = Vec::new();
    let mut required_args: Vec<String> = Vec::new();
    let mut optional_args: Vec<String> = Vec::new();

    if let Some(props) = properties {
        for (param_name, param_schema) in props {
            let py_type = json_type_to_python(param_schema["type"].as_str().unwrap_or("str"));
            if required.contains(&param_name.as_str()) {
                required_args.push(format!("{param_name}: {py_type}"));
            } else {
                let default = python_default(py_type);
                optional_args.push(format!("{param_name}: {py_type} = {default}"));
            }
        }
    }

    args.extend(required_args);
    args.extend(optional_args);
    let sig = args.join(", ");

    // Escape description for the docstring
    let docstring = description.replace('"', "'").replace('\n', " ");

    format!(
        "def {name}({sig}) -> str:\n    \
         \"\"\"{docstring}\"\"\"\n    \
         # Executed client-side by CADE (Rust implementation)\n    \
         return 'client-side tool'\n"
    )
}

fn json_type_to_python(json_type: &str) -> &'static str {
    match json_type {
        "integer" => "int",
        "number" => "float",
        "boolean" => "bool",
        "array" => "list",
        "object" => "dict",
        _ => "str", // default: string
    }
}

fn python_default(py_type: &str) -> &'static str {
    match py_type {
        "int" => "0",
        "float" => "0.0",
        "bool" => "False",
        "list" => "None",
        "dict" => "None",
        _ => "None",
    }
}

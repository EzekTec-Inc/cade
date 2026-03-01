use anyhow::Result;
use serde_json::{Value, json};

use super::client::{CreateToolRequest, CadeClient, ToolDef};
use crate::tools::schemas_for_toolset;
use crate::toolsets::Toolset;

/// Register all CADE tools with the CADE server.
///
/// The CADE server /v1/tools endpoint derives tool name + signature from the
/// Python source_code. We pass our JSON schema explicitly so the agent gets
/// accurate parameter descriptions.
///
/// Execution happens client-side in Rust — the Python stubs are never run.
pub async fn register_cade_tools(client: &CadeClient, toolset: Toolset) -> Result<Vec<ToolDef>> {
    // Fetch already-registered tools so we skip re-registration
    let existing = client.list_tools().await.unwrap_or_default();
    let existing_names: std::collections::HashSet<String> =
        existing.iter().map(|t| t.name.clone()).collect();

    let mut registered = Vec::new();

    for schema in schemas_for_toolset(toolset) {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();

        // Skip if already registered on the server
        if existing_names.contains(&name) {
            if let Some(t) = existing.iter().find(|t| t.name == name) {
                tracing::debug!("Tool '{}' already registered — reusing", name);
                registered.push(t.clone());
            }
            continue;
        }

        // Build a Python stub whose function name matches the tool name.
        // CADE server the tool name from `def <name>(...)`, so this must
        // be correct. Actual execution happens in Rust — the stub is never called.
        let stub = build_python_stub(&name, &description, &schema["parameters"]);

        // Full OpenAI-compatible JSON schema (the API accepts this to override
        // what it would auto-generate from the stub)
        let json_schema = json!({
            "name": name,
            "description": description,
            "parameters": schema["parameters"]
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
/// JSON schema properties. The function body returns a placeholder — Letta
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
        "number"  => "float",
        "boolean" => "bool",
        "array"   => "list",
        "object"  => "dict",
        _         => "str",  // default: string
    }
}

fn python_default(py_type: &str) -> &'static str {
    match py_type {
        "int"   => "0",
        "float" => "0.0",
        "bool"  => "False",
        "list"  => "None",
        "dict"  => "None",
        _       => "None",
    }
}

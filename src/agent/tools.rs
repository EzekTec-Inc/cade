use anyhow::Result;
use serde_json::json;

use super::client::{CreateToolRequest, LettaClient, ToolDef};
use crate::tools::all_schemas;

/// Register all CADE tools with the Letta server.
/// Uses a minimal Python stub as source_code — execution happens client-side.
/// Returns vec of registered ToolDef.
pub async fn register_cade_tools(client: &LettaClient) -> Result<Vec<ToolDef>> {
    let existing = client.list_tools().await.unwrap_or_default();
    let existing_names: std::collections::HashSet<String> =
        existing.iter().map(|t| t.name.clone()).collect();

    let mut registered = Vec::new();

    for schema in all_schemas() {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();

        if existing_names.contains(&name) {
            if let Some(t) = existing.iter().find(|t| t.name == name) {
                registered.push(t.clone());
            }
            continue;
        }

        // Minimal Python stub — actual execution is done client-side in Rust.
        // The stub is required by the Letta API but never called.
        let stub = format!(
            "def {name}(**kwargs) -> str:\n    \
             # Executed client-side by CADE\n    \
             return 'client-side tool'"
        );

        let req = CreateToolRequest {
            name: name.clone(),
            description,
            source_code: stub,
            source_type: "python".to_string(),
            json_schema: json!({
                "name": schema["name"],
                "description": schema["description"],
                "parameters": schema["parameters"]
            }),
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

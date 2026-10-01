use cade::toolsets::Toolset;
use cade_agent::agent;
use cade_agent::agent::HttpTransport;
use cade_core::capabilities::CapabilitySet;

/// Capability-aware tool registration: only registers and attaches tools
/// allowed by the given `CapabilitySet`.
pub async fn register_and_attach_with_caps(
    client: &HttpTransport,
    agent_id: &str,
    toolset: Toolset,
    caps: &CapabilitySet,
) {
    register_and_attach_with_caps_filtered(client, agent_id, toolset, caps, None).await;
}

/// Capability-aware + filter-aware registration. Meta tools remain available;
/// an explicit name list narrows native schemas before registration/attachment.
pub async fn register_and_attach_with_caps_filtered(
    client: &HttpTransport,
    agent_id: &str,
    toolset: Toolset,
    caps: &CapabilitySet,
    tool_filter: Option<&[String]>,
) {
    use agent::client::CreateToolRequest;
    use cade_agent::agent::tools::build_python_stub_from_schema as bps;
    use cade_agent::tools::catalog::{
        meta_schemas_for_capabilities, native_schemas_for_capabilities,
    };

    let meta_schemas = meta_schemas_for_capabilities(caps);
    let native_schemas: Vec<_> = native_schemas_for_capabilities(toolset, caps)
        .into_iter()
        .filter(|schema| {
            tool_filter.is_none_or(|names| {
                schema["name"]
                    .as_str()
                    .is_some_and(|name| names.iter().any(|n| n == name))
            })
        })
        .collect();

    let mut ids = Vec::new();

    // Register meta tools
    for schema in &meta_schemas {
        let req = CreateToolRequest {
            source_code: String::new(),
            source_type: "json".to_string(),
            json_schema: Some(schema.clone()),
            tags: vec!["cade".to_string(), "meta".to_string()],
        };
        match client.create_tool(req).await {
            Ok(tool) => ids.push(tool.id),
            Err(e) => tracing::debug!("meta tool registration: {e}"),
        }
    }

    // Register native tools
    for schema in &native_schemas {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();
        let stub = bps(&name, &description, &schema["parameters"]);
        let req = CreateToolRequest {
            source_code: stub,
            source_type: "python".to_string(),
            json_schema: Some(schema.clone()),
            tags: vec!["cade".to_string()],
        };
        match client.create_tool(req).await {
            Ok(tool) => ids.push(tool.id),
            Err(e) => tracing::warn!("register tool '{name}': {e}"),
        }
    }

    tracing::info!(
        "Registered {} tools ({} meta + {} native) for profile",
        ids.len(),
        meta_schemas.len(),
        native_schemas.len()
    );

    #[allow(clippy::collapsible_if)]
    if !ids.is_empty() {
        if let Err(e) = client.attach_agent_tools(agent_id, &ids).await {
            tracing::warn!("attach_agent_tools: {e}");
        }
    }
}

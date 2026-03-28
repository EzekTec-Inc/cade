use cade_agent::agent::CadeClient;
use cade_agent::agent;
use cade::toolsets::Toolset;
use cade::tools::schemas_for_names as agent_schemas_for_names;
use cade_agent::agent::tools::register_cade_tools;
use cade::{Result, Error};
use cade_core::capabilities::CapabilitySet;

/// Register all CADE tools on the server and attach them to the given agent.
pub async fn register_and_attach(client: &CadeClient, agent_id: &str, toolset: Toolset) {
    register_and_attach_filtered(client, agent_id, toolset, None).await;
}

/// Capability-aware tool registration: only registers and attaches tools
/// allowed by the given `CapabilitySet`.
pub async fn register_and_attach_with_caps(
    client: &CadeClient,
    agent_id: &str,
    toolset: Toolset,
    caps: &CapabilitySet,
) {
    use cade_agent::tools::catalog::{meta_schemas_for_capabilities, native_schemas_for_capabilities};
    use agent::client::CreateToolRequest;
    use cade_agent::agent::tools::build_python_stub_from_schema as bps;

    let meta_schemas = meta_schemas_for_capabilities(caps);
    let native_schemas = native_schemas_for_capabilities(toolset, caps);

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

    if !ids.is_empty() {
        if let Err(e) = client.attach_agent_tools(agent_id, &ids).await {
            tracing::warn!("attach_agent_tools: {e}");
        }
    }
}

/// Register CADE tools, optionally restricted to a name list, and attach to agent.
/// `tool_filter = None`   → attach all tools for toolset
/// `tool_filter = Some([])` → attach only meta-tools (memory/skills/subagents; not filtered)
/// `tool_filter = Some(names)` → attach only tools whose name is in `names`
pub async fn register_and_attach_filtered(
    client: &CadeClient,
    agent_id: &str,
    toolset: Toolset,
    tool_filter: Option<&[String]>,
) {
    // Register all meta tools (memory, skills, subagents) via the centralised registry.
    let meta_ids = cade_agent::tools::register_meta_tools(client).await;
    let tools = register_cade_tools_filtered(client, toolset, tool_filter)
        .await
        .unwrap_or_default();
    let mut ids: Vec<String> = tools.iter().map(|t| t.id.clone()).collect();
    ids.extend(meta_ids);
    tracing::info!("Registered {} native tools", tools.len());
    if !ids.is_empty()
        && let Err(e) = client.attach_agent_tools(agent_id, &ids).await
    {
        tracing::warn!("attach_agent_tools: {e}");
    }
}

pub async fn register_cade_tools_filtered(
    client: &CadeClient,
    toolset: Toolset,
    filter: Option<&[String]>,
) -> Result<Vec<agent::client::ToolDef>> {
    // schemas_for_toolset and schemas_for_names imported at top-level
    // When no filter, use normal registration path
    let Some(names) = filter else {
        return register_cade_tools(client, toolset)
            .await
            .map_err(Error::Agent);
    };
    let schemas = if names.is_empty() {
        // Empty filter → no tools (analysis-only mode)
        vec![]
    } else {
        agent_schemas_for_names(toolset, names)
    };

    // Reuse the existing tool registration logic by passing schemas directly
    use agent::client::CreateToolRequest;
    use agent::tools::build_python_stub_from_schema as bps;
    let existing = client.list_tools().await.unwrap_or_default();
    let existing_map: std::collections::HashMap<String, agent::client::ToolDef> =
        existing.into_iter().map(|t| (t.name.clone(), t)).collect();

    let mut registered = Vec::new();
    for schema in schemas {
        let name = schema["name"].as_str().unwrap_or("").to_string();
        let description = schema["description"].as_str().unwrap_or("").to_string();
        if let Some(t) = existing_map.get(&name) {
            registered.push(t.clone());
            continue;
        }
        let stub = bps(&name, &description, &schema["parameters"]);
        let req = CreateToolRequest {
            source_code: stub,
            source_type: "python".to_string(),
            json_schema: Some(schema),
            tags: vec!["cade".to_string()],
        };
        match client.create_tool(req).await {
            Ok(t) => registered.push(t),
            Err(e) => tracing::warn!("register filtered tool '{name}': {e}"),
        }
    }
    Ok(registered)
}

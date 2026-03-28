use cade_agent::agent::CadeClient;
use cade_agent::agent;
use cade::toolsets::Toolset;
use cade::tools::schemas_for_names as agent_schemas_for_names;
use cade_agent::agent::tools::register_cade_tools;
use cade::{Result, Error};

/// Register all CADE tools on the server and attach them to the given agent.
pub async fn register_and_attach(client: &CadeClient, agent_id: &str, toolset: Toolset) {
    register_and_attach_filtered(client, agent_id, toolset, None).await;
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

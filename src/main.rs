// Re-use modules declared in lib.rs
use cade::agent;
use cade::cli;
use cade::desktop;
use cade::permissions;
use cade::settings;
use cade::skills;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::PathBuf;

use std::sync::{Arc, Mutex};

use agent::{
    CadeClient,
    client::{CreateAgentRequest, MemoryBlock},
    session::SessionStore,
    tools::register_cade_tools,
};
use cli::{Args, Repl};
use permissions::{PermissionManager, PermissionMode};
use settings::SettingsManager;
use skills::{discover_all_skills, skills_listing};

const SKILLS_DIR: &str = ".skills";

/// Default memory block labels and their seed values.
const DEFAULT_MEMORY_BLOCKS: &[(&str, &str)] = &[
    ("persona",  "CADE is a coding AI assistant with desktop extensions. It helps with programming tasks, file management, shell commands, and desktop automation. CADE prefers concise, accurate responses and always verifies changes before reporting success."),
    ("human",    ""),   // agent fills in as it learns about the user
    ("project",  ""),   // agent fills in via /init
];

/// Seed default memory blocks for a newly created agent.
async fn seed_default_memory(client: &CadeClient, agent_id: &str) {
    for (label, value) in DEFAULT_MEMORY_BLOCKS {
        if let Err(e) = client.upsert_memory(agent_id, label, value).await {
            tracing::warn!("seed_memory {label}: {e}");
        }
    }
}

/// Register the `load_skill` tool that lets the agent load skill content on-demand.
async fn register_load_skill_tool(client: &CadeClient) {
    let schema = serde_json::json!({
        "name": "load_skill",
        "description": "Load the full content of a skill into context. Call this when starting a task that matches one of the available skills listed in your system prompt.",
        "input_schema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The skill ID to load (from the Available Skills list)"
                }
            },
            "required": ["id"]
        }
    });
    use agent::client::CreateToolRequest;
    let req = CreateToolRequest {
        source_code: String::new(),
        source_type: "json".to_string(),
        json_schema: Some(schema),
        tags: vec![],
    };
    if let Err(e) = client.create_tool(req).await {
        tracing::debug!("load_skill tool: {e}");
    }
}

/// Register the `install_skill` tool that lets the agent install skills from URLs.
async fn register_install_skill_tool(client: &CadeClient) {
    let schema = serde_json::json!({
        "name": "install_skill",
        "description": "Download and install a skill from a GitHub URL or direct SKILL.MD URL. Use when the user asks to install a skill.",
        "input_schema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "GitHub tree URL or direct SKILL.MD URL to install"
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "Where to install: project (.skills/) or global (~/.cade/skills/)"
                }
            },
            "required": ["url"]
        }
    });
    use agent::client::CreateToolRequest;
    let req = CreateToolRequest {
        source_code: String::new(),
        source_type: "json".to_string(),
        json_schema: Some(schema),
        tags: vec![],
    };
    if let Err(e) = client.create_tool(req).await {
        tracing::debug!("install_skill tool: {e}");
    }
}

/// Register the `update_memory` tool that lets the agent update its own memory.
async fn register_update_memory_tool(client: &CadeClient) {
    let schema = serde_json::json!({
        "name": "update_memory",
        "description": "Update a persistent memory block. Use this to store important information about the user, project, or yourself that should be remembered across conversations. Call this whenever you learn something worth remembering.",
        "input_schema": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block name: 'human' (user info), 'project' (project context), 'persona' (your identity/style), or any custom label"
                },
                "value": {
                    "type": "string",
                    "description": "Content to store in the memory block"
                },
                "operation": {
                    "type": "string",
                    "enum": ["set", "append"],
                    "description": "set = replace the block entirely, append = add to existing content"
                }
            },
            "required": ["label", "value"]
        }
    });
    use agent::client::CreateToolRequest;
    let req = CreateToolRequest {
        source_code: String::new(),
        source_type: "json".to_string(),
        json_schema: Some(schema),
        tags: vec![],
    };
    if let Err(e) = client.create_tool(req).await {
        tracing::debug!("update_memory tool already registered or failed: {e}");
    }
}

/// Register all CADE tools on the server and attach them to the given agent.
async fn register_and_attach(client: &CadeClient, agent_id: &str) {
    register_update_memory_tool(client).await;
    register_load_skill_tool(client).await;
    register_install_skill_tool(client).await;
    let tools = register_cade_tools(client).await.unwrap_or_default();
    let ids: Vec<String> = tools.iter().map(|t| t.id.clone()).collect();
    tracing::info!("Registered {} tools", tools.len());
    if !ids.is_empty() {
        if let Err(e) = client.attach_agent_tools(agent_id, &ids).await {
            tracing::warn!("attach_agent_tools: {e}");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let _ = dotenvy::dotenv();

    let args = Args::parse();
    let cwd = std::env::current_dir().context("get cwd")?;

    // Settings + session
    let mut settings = SettingsManager::new(&cwd).context("load settings")?;
    let mut session = SessionStore::load(&cwd);

    // API credentials
    let api_key = settings
        .api_key()
        .context("No CADE_API_KEY. Set via env var or ~/.cade/settings.json")?;
    let base_url = settings.base_url();

    let client = CadeClient::new(base_url.clone(), api_key).context("create CADE server")?;

    if !client.health().await.unwrap_or(false) {
        bail!("Cannot connect to CADE server at {base_url}. Check CADE_API_KEY and CADE_SERVER_URL.");
    }

    // Fetch server's detected provider + model — shown in banner + used for agent creation
    let server_info = {
        let resp = client.server_default_model().await;
        // server_default_model returns "provider/model" — split for display
        let (prov, mdl) = resp.split_once('/').unwrap_or(("unknown", &resp));
        (prov.to_string(), mdl.to_string(), resp)
    };
    eprintln!(
        "Connected to cade-server at {base_url} | provider={} | model={}",
        server_info.0, server_info.1
    );

    // Permissions
    let perm_mode: PermissionMode = args
        .effective_permission_mode()
        .parse()
        .context("invalid permission mode")?;
    let permissions = PermissionManager::new(perm_mode);

    // Skills — multi-scope discovery: project > agent > global
    let skills_dir = args
        .skills
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(SKILLS_DIR));
    let loaded_skills = discover_all_skills(&cwd, None, None);
    if !loaded_skills.is_empty() {
        println!("Loaded {} skill(s)", loaded_skills.len());
    }

    // Default model: CLI flag > CADE_DEFAULT_MODEL env > server's detected model
    let default_model = args.model.clone()
        .or_else(|| std::env::var("CADE_DEFAULT_MODEL").ok())
        .unwrap_or(server_info.2);

    // Skills listing — compact (names + descriptions only), not full bodies.
    // The agent uses load_skill(id) to pull full content on-demand.
    let skills_block = skills_listing(&loaded_skills);

    // Agent resolution — helper closure avoids repeating the create logic
    let make_req = |model: String, desc: &str| {
        // Inject only the compact listing as a memory block.
        // Full skill content is loaded on-demand by the agent via load_skill tool.
        let memory_blocks: Vec<MemoryBlock> = if let Some(ctx) = &skills_block {
            vec![MemoryBlock { label: "skills".to_string(), value: ctx.clone(), description: None }]
        } else {
            vec![]
        };
        CreateAgentRequest {
            name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
            model,
            description: Some(desc.to_string()),
            memory_blocks,
            tool_ids: vec![],
        }
    };



    let agent = if args.new_agent {
        let a = client
            .create_agent(make_req(default_model.clone(), "CADE coding agent with desktop extensions"))
            .await
            .context("create agent")?;
        register_and_attach(&client, &a.id).await;
        seed_default_memory(&client, &a.id).await;
        session.set_agent(a.id.clone(), Some(a.name.clone())).context("save session")?;
        settings.set_last_agent(&a.id).context("save global session")?;
        a
    } else if let Some(id) = &args.agent {
        client.get_agent(id).await.with_context(|| format!("get agent {id}"))?
    } else if let Some(last_id) = session.session.agent_id.clone() {
        match client.get_agent(&last_id).await {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Previous agent {last_id} not found — creating new agent");
                let a = client
                    .create_agent(make_req(default_model.clone(), "CADE coding agent"))
                    .await
                    .context("create agent")?;
                register_and_attach(&client, &a.id).await;
                seed_default_memory(&client, &a.id).await;
                session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                settings.set_last_agent(&a.id)?;
                a
            }
        }
    } else {
        println!("No previous session — creating new agent…");
        let a = client
            .create_agent(make_req(default_model.clone(), "CADE coding agent with desktop extensions"))
            .await
            .context("create agent")?;
        register_and_attach(&client, &a.id).await;
        seed_default_memory(&client, &a.id).await;
        session.set_agent(a.id.clone(), Some(a.name.clone()))?;
        settings.set_last_agent(&a.id)?;
        a
    };

    // Tray
    if args.tray {
        match desktop::spawn_tray() {
            Ok(_) => tracing::info!("System tray started"),
            Err(e) => tracing::warn!("System tray failed: {e}"),
        }
    }

    // Headless
    if let Some(prompt) = &args.prompt {
        let output = cli::headless::run_headless(&client, &agent.id, prompt, &permissions).await?;
        println!("{output}");
        return Ok(());
    }

    // Info
    if args.info {
        println!("CADE v{}", env!("CARGO_PKG_VERSION"));
        println!("Agent   : {} ({})", agent.name, agent.id);
        println!("Server  : {base_url}");
        println!("Model   : {default_model}");
        println!("Mode    : {}", permissions.mode());
        println!("CWD     : {}", cwd.display());
        println!("Skills  : {}", loaded_skills.len());
        return Ok(());
    }

    // Interactive REPL
    let settings_arc = Arc::new(Mutex::new(settings));
    let session_arc  = Arc::new(Mutex::new(session));
    let repl = Repl::new(
        client,
        agent.id,
        agent.name,
        permissions,
        default_model.clone(),
        settings_arc,
        session_arc,
        cwd.clone(),
        loaded_skills,
        skills_dir,
    );
    repl.run().await?;

    Ok(())
}

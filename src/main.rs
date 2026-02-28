mod agent;
mod cli;
mod desktop;
mod permissions;
mod settings;
mod skills;
mod tools;

use anyhow::{Context, Result, bail};
use clap::Parser;

use agent::{
    LettaClient,
    client::CreateAgentRequest,
    session::SessionStore,
    tools::register_cade_tools,
};
use cli::{Args, Repl};
use permissions::{PermissionManager, PermissionMode};
use settings::SettingsManager;

const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250929";

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging (stderr only, so stdout stays clean)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // Load .env if present
    let _ = dotenvy::dotenv();

    let args = Args::parse();
    let cwd = std::env::current_dir().context("get cwd")?;

    // Settings + session
    let mut settings = SettingsManager::new(&cwd).context("load settings")?;
    let mut session = SessionStore::load(&cwd);

    // Resolve API key
    let api_key = settings.api_key()
        .context("No LETTA_API_KEY found. Set via env var or ~/.cade/settings.json")?;
    let base_url = settings.base_url();

    // Build Letta client
    let client = LettaClient::new(base_url.clone(), api_key)
        .context("create Letta client")?;

    // Verify connectivity
    if !client.health().await.unwrap_or(false) {
        bail!("Cannot connect to Letta server at {base_url}. Check LETTA_API_KEY and LETTA_BASE_URL.");
    }

    // Resolve permission mode
    let perm_mode: PermissionMode = args
        .effective_permission_mode()
        .parse()
        .context("invalid permission mode")?;
    let permissions = PermissionManager::new(perm_mode);

    // Resolve or create agent
    let agent = if args.new_agent {
        // Always create a new agent
        let model = args.model.as_deref().unwrap_or(DEFAULT_MODEL).to_string();
        let agent = client.create_agent(CreateAgentRequest {
            name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
            model,
            description: Some("CADE coding agent with desktop extensions".to_string()),
            memory_blocks: vec![],
            tool_ids: vec![],
        }).await.context("create agent")?;

        // Register tools and attach
        let tools = register_cade_tools(&client).await
            .context("register tools")?;
        tracing::info!("Registered {} tools", tools.len());

        session.set_agent(agent.id.clone(), Some(agent.name.clone()))
            .context("save session")?;
        settings.set_last_agent(&agent.id).context("save global session")?;

        agent
    } else if let Some(agent_id) = &args.agent {
        client.get_agent(agent_id).await
            .with_context(|| format!("get agent {agent_id}"))?
    } else if let Some(last_id) = session.session.agent_id.clone() {
        // Resume last agent for this project
        match client.get_agent(&last_id).await {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Previous agent {last_id} not found. Creating new agent...");
                let model = args.model.as_deref().unwrap_or(DEFAULT_MODEL).to_string();
                let agent = client.create_agent(CreateAgentRequest {
                    name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
                    model,
                    description: Some("CADE coding agent".to_string()),
                    memory_blocks: vec![],
                    tool_ids: vec![],
                }).await.context("create agent")?;
                let _ = register_cade_tools(&client).await;
                session.set_agent(agent.id.clone(), Some(agent.name.clone()))?;
                agent
            }
        }
    } else {
        // No previous session — create new agent
        let model = args.model.as_deref().unwrap_or(DEFAULT_MODEL).to_string();
        println!("No previous session found. Creating new agent...");
        let agent = client.create_agent(CreateAgentRequest {
            name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
            model,
            description: Some("CADE coding agent with desktop extensions".to_string()),
            memory_blocks: vec![],
            tool_ids: vec![],
        }).await.context("create agent")?;

        let tools = register_cade_tools(&client).await?;
        tracing::info!("Registered {} tools", tools.len());

        session.set_agent(agent.id.clone(), Some(agent.name.clone()))?;
        settings.set_last_agent(&agent.id)?;
        agent
    };

    // Optional: spawn tray if requested
    if args.tray {
        match desktop::spawn_tray() {
            Ok(_tx) => tracing::info!("System tray started"),
            Err(e) => tracing::warn!("System tray failed to start: {e}"),
        }
    }

    // Headless mode
    if let Some(prompt) = &args.prompt {
        let output = cli::headless::run_headless(
            &client,
            &agent.id,
            prompt,
            &permissions,
        ).await?;
        println!("{output}");
        return Ok(());
    }

    // Info mode
    if args.info {
        println!("CADE v{}", env!("CARGO_PKG_VERSION"));
        println!("Agent: {} ({})", agent.name, agent.id);
        println!("Server: {base_url}");
        println!("Permission mode: {}", permissions.mode());
        println!("CWD: {}", cwd.display());
        return Ok(());
    }

    // Interactive REPL
    let repl = Repl::new(client, agent.id, agent.name, permissions);
    repl.run().await?;

    Ok(())
}

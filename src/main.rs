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

use agent::{
    CadeClient,
    client::CreateAgentRequest,
    session::SessionStore,
    tools::register_cade_tools,
};
use cli::{Args, Repl};
use permissions::{PermissionManager, PermissionMode};
use settings::SettingsManager;
use skills::discover_skills;

const SKILLS_DIR: &str = ".skills";

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

    // Skills — discovered from .skills/ in cwd (or custom dir)
    let skills_dir = args
        .skills
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(SKILLS_DIR));
    let loaded_skills = discover_skills(&skills_dir).unwrap_or_default();
    if !loaded_skills.is_empty() {
        println!("Loaded {} skill(s) from {}", loaded_skills.len(), skills_dir.display());
    }

    // Default model: CLI flag > CADE_DEFAULT_MODEL env > server's detected model
    let default_model = args.model.clone()
        .or_else(|| std::env::var("CADE_DEFAULT_MODEL").ok())
        .unwrap_or(server_info.2);

    // Agent resolution — helper closure avoids repeating the create logic
    let make_req = |model: String, desc: &str| CreateAgentRequest {
        name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
        model,
        description: Some(desc.to_string()),
        memory_blocks: vec![],
        tool_ids: vec![],
    };

    let agent = if args.new_agent {
        let a = client
            .create_agent(make_req(default_model.clone(), "CADE coding agent with desktop extensions"))
            .await
            .context("create agent")?;
        let tools = register_cade_tools(&client).await.context("register tools")?;
        tracing::info!("Registered {} tools", tools.len());
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
                let _ = register_cade_tools(&client).await;
                session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                settings.set_last_agent(&a.id)?;
                a
            }
        }
    } else {
        println!("No previous session — creating new agent…");
        let model = default_model.clone();
        let a = client
            .create_agent(make_req(model, "CADE coding agent with desktop extensions"))
            .await
            .context("create agent")?;
        let tools = register_cade_tools(&client).await.context("register tools")?;
        tracing::info!("Registered {} tools", tools.len());
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
    let repl = Repl::new(client, agent.id, agent.name, permissions);
    repl.run().await?;

    Ok(())
}

#![allow(clippy::too_many_arguments)]
// region:    --- Modules

// Re-use modules declared in lib.rs
use cade::agent;
use cade::cli;
#[cfg(feature = "desktop")]
use cade::desktop;
use cade::hooks::HookEngine;
use cade::mcp::McpManager;
use cade::permissions;
use cade::settings;
use cade::skills;

use cade::{Error, Result};
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;

use std::sync::Arc;
use parking_lot::Mutex;

use agent::{CadeClient, session::SessionStore};
use cade::support::text::sanitize_for_terminal;
use cade::toolsets::Toolset;
use cli::{Args, EvalAction, PackageAction, PackageSubcommand, Repl};
use permissions::{PermissionManager, PermissionMode};
use settings::SettingsManager;
use skills::{discover_all_skills, skills_listing};

// endregion: --- Modules

const SKILLS_DIR: &str = ".skills";

mod bootstrap;
use bootstrap::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Write tracing to a log file instead of stderr.  In alternate-screen TUI
    // mode crossterm only redirects stdout to the alt buffer — stderr writes go
    // directly to the terminal at the current cursor position (the input field),
    // corrupting the display.  Fall back to discarding logs if the file can't
    // be opened.
    let log_writer: Box<dyn std::io::Write + Send + Sync> = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cade.log")
    {
        Ok(f) => Box::new(f),
        Err(_) => Box::new(std::io::sink()),
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::sync::Mutex::new(log_writer))
        .with_ansi(false)
        .init();

    // Install a panic hook that restores the terminal before printing the
    // panic message.  Without this, a panic while the TUI is active leaves
    // the terminal in raw/alternate-screen mode: the message is invisible or
    // garbled and the shell prompt is corrupted.
    //
    // ratatui::restore() disables raw mode and leaves the alternate screen.
    // We capture the original hook so the panic message + backtrace are still
    // printed normally after the terminal is restored.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort — ignore errors (we are already panicking).
        ratatui::restore();
        original_hook(info);
    }));

    let _ = dotenvy::dotenv();

    let mut args = Args::parse();
    let cwd = std::env::current_dir().map_err(|e| Error::custom(format!("get cwd: {e}")))?;

    // Agent config directory: $CADE_AGENT_DIR or ~/.cade
    let agent_dir: std::path::PathBuf = std::env::var("CADE_AGENT_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let home: Option<std::path::PathBuf> = dirs::home_dir();
            home.map(|h| h.join(".cade"))
        })
        .unwrap_or_else(|| cwd.join(".cade"));

    // Settings + session
    let mut settings =
        SettingsManager::new(&cwd).map_err(|e| Error::custom(format!("load settings: {e}")))?;

    // -- Package subcommand (runs before server connection, no server needed)
    let is_eval_subcommand = matches!(&args.package, Some(PackageSubcommand::Eval { .. }));
    if let Some(PackageSubcommand::Package { action }) = args.package.take() {
        match action {
            PackageAction::Install {
                source,
                project_local,
            } => {
                cade::cli::package::cmd_install(
                    &source,
                    project_local,
                    &mut settings,
                    &cwd,
                    &agent_dir,
                )
                .await
                .map_err(|e| Error::custom(format!("package install: {e}")))?;
            }
            PackageAction::Remove { source } => {
                cade::cli::package::cmd_remove(&source, &agent_dir)
                    .map_err(|e| Error::custom(format!("package remove: {e}")))?;
            }
            PackageAction::List => {
                cade::cli::package::cmd_list(&agent_dir)
                    .map_err(|e| Error::custom(format!("package list: {e}")))?;
            }
            PackageAction::Update => {
                cade::cli::package::cmd_update(&agent_dir)
                    .await
                    .map_err(|e| Error::custom(format!("package update: {e}")))?;
            }
        }
        return Ok(());
    }
    // Eval subcommand deferred — needs server connection (handled after agent resolution below)
    let mut session = SessionStore::load(&cwd);

    // API credentials
    let api_key = settings.api_key().ok_or_else(|| {
        Error::custom("No CADE_API_KEY. Set via env var or ~/.cade/settings.json")
    })?;
    let base_url = settings.base_url();

    let client = CadeClient::new(base_url.clone(), api_key)
        .map_err(|e| Error::custom(format!("create CADE server: {e}")))?;

    if !client.health().await.unwrap_or(false) {
        auto_start_server(&base_url).await?;
    }

    // Version compatibility check: warn if client and server versions differ.
    // Mismatched versions can cause subtle protocol or schema issues.
    if let Some(srv_ver) = client.server_version().await {
        let cli_ver = env!("CARGO_PKG_VERSION");
        if srv_ver != cli_ver {
            tracing::warn!(
                "Version mismatch: client={cli_ver}, server={srv_ver}. \
                 Consider restarting cade-server."
            );
        }
    }

    // Push any API keys from the CLI's environment to the server.
    //
    // cade-server is a separate process and only sees its own startup environment.
    // If the user exports API keys in the terminal where `cade` runs (after the server
    // started), the server's hot_sync_env_providers() won't find them.  By calling
    // POST /v1/providers here the CLI forwards its keys, letting the server register
    // the providers and fetch live model lists without requiring a server restart.
    //
    // Errors are silently ignored — if the provider is already registered with the same
    // key, the server upserts cleanly; if the key is invalid, the user will see errors
    // when they actually try to use the model (not here at startup).
    push_env_providers_to_server(&client).await;

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
        .map_err(|e| Error::custom(format!("invalid permission mode: {e}")))?;
    let strict_bash = settings.permission_settings().strict_bash;
    let permissions = PermissionManager::new_with_strict_bash(perm_mode, strict_bash);

    // Load persistent rules from ~/.cade/settings.json
    for raw in &settings.permission_settings().allow.clone() {
        if let Some(rule) = permissions::PermissionRule::parse(raw) {
            permissions.add_allow_rule(rule);
        }
    }
    for raw in &settings.permission_settings().deny.clone() {
        if let Some(rule) = permissions::PermissionRule::parse(raw) {
            permissions.add_deny_rule(rule);
        }
    }
    // Load CLI flag rules (override / supplement settings)
    if let Some(s) = &args.allowed_tools {
        for raw in s.split(',') {
            if let Some(rule) = permissions::PermissionRule::parse(raw.trim()) {
                permissions.add_allow_rule(rule);
            }
        }
    }
    if let Some(s) = &args.disallowed_tools {
        for raw in s.split(',') {
            if let Some(rule) = permissions::PermissionRule::parse(raw.trim()) {
                permissions.add_deny_rule(rule);
            }
        }
    }

    // -- Capability profile resolution
    // CLI flag > env var > settings file > default (Full)
    let capabilities = {
        use cade_core::capabilities::resolve_capabilities;
        let caps = resolve_capabilities(
            &settings.global().enable_capabilities,
            &settings.global().disable_capabilities,
        );
        tracing::info!("Capabilities enabled: {}", caps.len());
        caps
    };

    // Skills — multi-scope discovery: project > agent > global
    let skills_dir = args
        .skills
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(SKILLS_DIR));
    let initial_loaded_skills = discover_all_skills(&cwd, None, None);

    // Default model: CLI flag > CADE_DEFAULT_MODEL env > server's detected model
    let default_model = args
        .model
        .clone()
        .or_else(|| std::env::var("CADE_DEFAULT_MODEL").ok())
        .unwrap_or(server_info.2);

    // Detect toolset: --toolset flag > model family auto-detection
    let toolset = args
        .toolset
        .as_deref()
        .and_then(Toolset::from_name)
        .unwrap_or_else(|| Toolset::for_model(&default_model));
    tracing::info!(
        "Toolset: {} (model={})",
        toolset.display_name(),
        default_model
    );

    // Skills listing — compact (names + descriptions only), not full bodies.
    // The agent uses load_skill(id) to pull full content on-demand.
    let skills_block = skills_listing(&initial_loaded_skills);

    // Agent resolution — helper closure avoids repeating the create logic
    let (agent, loaded_skills, conversation_id, effective_system_prompt) =
        resolve_agent_and_conversation(
            &client,
            &args,
            &default_model,
            toolset,
            &skills_block,
            &cwd,
            &agent_dir,
            &mut session,
            &mut settings,
            &capabilities,
        )
        .await?;

    // -- MCP server startup (skipped when Mcp capability is off)
    let mcp_configs = settings.merged_mcp_servers();
    let mcp_enabled = capabilities.is_enabled(cade_core::capabilities::Capability::Mcp);
    let mcp: std::sync::Arc<McpManager> = if mcp_configs.is_empty() || !mcp_enabled {
        if !mcp_configs.is_empty() && !mcp_enabled {
            tracing::info!(
                "Skipping {} MCP server(s) — Mcp capability not enabled",
                mcp_configs.len()
            );
        }
        std::sync::Arc::new(McpManager::empty())
    } else {
        println!("Starting {} MCP server(s)…", mcp_configs.len());
        let mgr = McpManager::start(&mcp_configs).await;
        let count = mgr.status().await.len();
        let total = mcp_configs.len();
        if count == 0 {
            eprintln!("Warning: no MCP servers started successfully");
        } else {
            println!("MCP: {count}/{total} server(s) ready");
        }
        std::sync::Arc::new(mgr)
    };

    // Sync MCP tools every startup: remove stale MCP tool entries (from removed
    // or disconnected servers) while preserving native/meta tool attachments.
    //
    // Strategy: MCP tools always have "__" in their name (server__tool prefix);
    // native and meta tools never do.  Snapshot the current non-MCP tool IDs,
    // detach everything, re-attach non-MCP IDs immediately, then let the block
    // below re-attach only this session's live MCP tools.
    {
        let non_mcp_ids: Vec<String> = client
            .get_agent_tools(&agent.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, name)| !name.contains("__"))
            .map(|(id, _)| id)
            .collect();
        let _ = client.detach_agent_tools(&agent.id).await;
        if !non_mcp_ids.is_empty() {
            let _ = client.attach_agent_tools(&agent.id, &non_mcp_ids).await;
        }
    }

    // Register MCP tool schemas with cade-server + attach to agent.
    // Only runs when at least one MCP server is live this session.
    if !mcp.is_empty().await {
        use agent::tools::register_mcp_tools;
        let mcp_tool_ids: Vec<String> = register_mcp_tools(&client, mcp.all_tool_schemas().await)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.id)
            .collect();

        if !mcp_tool_ids.is_empty() {
            if let Err(e) = client.attach_agent_tools(&agent.id, &mcp_tool_ids).await {
                tracing::warn!("Failed to attach MCP tools to agent: {e}");
            } else {
                println!("Attached {} MCP tool(s) to agent", mcp_tool_ids.len());
            }
        }
    }

    // Build hook engine from merged settings (local > project > global)
    let hook_engine = HookEngine::new(settings.merged_hooks(), cwd.clone());
    if !hook_engine.is_empty() {
        tracing::info!("Hooks loaded from settings");
    }

    // Expose AGENT_ID to all child processes (bash tool, hooks, etc.) without touching
    // global process env APIs (unsafe in Rust 2024).
    cade_core::agent_env::set_agent_id(agent.id.clone());

    // --unlink: detach all tools from agent, then continue
    if args.unlink {
        match client.detach_agent_tools(&agent.id).await {
            Ok(n) => println!("✓ Detached {n} tool(s) from agent"),
            Err(e) => eprintln!("Warning: detach failed: {e}"),
        }
    }

    // --link: (re-)attach native + MCP tools to agent, then continue
    if args.link {
        register_and_attach_with_caps(&client, &agent.id, toolset, &capabilities).await;
        if !mcp.is_empty().await {
            use agent::tools::register_mcp_tools;
            let mcp_ids: Vec<String> = register_mcp_tools(&client, mcp.all_tool_schemas().await)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.id)
                .collect();
            if !mcp_ids.is_empty() {
                let _ = client.attach_agent_tools(&agent.id, &mcp_ids).await;
            }
        }
        println!("✓ Tools linked to agent");
    }

    // --rename <new-name>: rename the resolved agent and exit (no REPL)
    if let Some(new_name) = &args.rename {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            eprintln!("✗ --rename: name cannot be empty");
            std::process::exit(1);
        }
        match client.rename_agent(&agent.id, new_name).await {
            Ok(_) => println!("✓ Renamed '{}' → '{new_name}'  ({})", agent.name, agent.id),
            Err(e) => {
                eprintln!("✗ {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // Migrate old system prompt: if the stored prompt is the minimal server fallback
    // (no "Never introduce yourself" rule) update it to effective_system_prompt.
    // This runs once per old agent; after the update the check is skipped.
    if agent
        .system_prompt
        .as_deref()
        .map(|p| !p.contains("Never introduce yourself") || !p.contains("No rule acknowledgment"))
        .unwrap_or(true)
    {
        if let Err(e) = client
            .patch_agent_system_prompt(&agent.id, &effective_system_prompt)
            .await
        {
            tracing::warn!("migrate system_prompt: {e}");
        } else {
            tracing::info!("Migrated system_prompt for agent {}", agent.id);
        }
    }

    // Seed default memory blocks if this agent has none yet
    // (covers agents created before default block seeding was introduced)
    let existing_blocks = client.get_memory(&agent.id).await.unwrap_or_default();
    if existing_blocks.is_empty() {
        seed_default_memory(&client, &agent.id).await;
    } else {
        // Migrate old persona blocks that describe CADE in a way that triggers
        // self-introductions: third-person ("CADE is…") or first-person intro
        // ("I am CADE…").  Replace with behavioral first-person phrasing.
        for block in &existing_blocks {
            if block.label == "persona" {
                let v = block.value.trim_start();
                let needs_migration = v.starts_with("CADE is") || v.starts_with("I am CADE");
                if needs_migration {
                    let (_, new_val, new_desc, _, _) = cade::DEFAULT_MEMORY_BLOCKS[0]; // persona entry
                    let _ = client
                        .upsert_memory(&agent.id, "persona", new_val, Some(new_desc))
                        .await;
                }
            }

            // Ensure core blocks are pinned so they are never auto-archived.
            // working_set intentionally stays as short-tier (it should age out
            // when stale); session_summary is managed by the Sleeptime task.
            if matches!(block.label.as_str(), "persona" | "human" | "project")
                && block.tier.as_deref() != Some("pinned")
            {
                let _ = client
                    .set_memory_tier(&agent.id, &block.label, "pinned")
                    .await;
            }
        }
    }

    // Tray
    #[cfg(feature = "desktop")]
    if args.tray {
        match desktop::spawn_tray() {
            Ok(_) => tracing::info!("System tray started"),
            Err(e) => tracing::warn!("System tray failed: {e}"),
        }
    }

    // Headless — --prompt flag OR piped stdin
    let piped_stdin: Option<String> = if !atty::is(atty::Stream::Stdin) {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).ok();
        let s = buf.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    } else {
        None
    };

    // -- Eval subcommand (needs server + agent)
    if is_eval_subcommand {
        if let Some(PackageSubcommand::Eval { action }) = args.package.take() {
            match action {
                EvalAction::List => {
                    cade::cli::eval::cmd_list(&client)
                        .await
                        .map_err(|e| Error::custom(format!("eval list: {e}")))?;
                }
                EvalAction::Show { id } => {
                    cade::cli::eval::cmd_show(&client, &id)
                        .await
                        .map_err(|e| Error::custom(format!("eval show: {e}")))?;
                }
                EvalAction::Run { task, model } => {
                    let result = cade::cli::eval::cmd_run(&client, &task, model.as_deref(), &cwd)
                        .await
                        .map_err(|e| Error::custom(format!("eval run: {e}")))?;
                    result.print_summary();
                }
                EvalAction::Bench {
                    dir,
                    model,
                    concurrency,
                } => {
                    cade::cli::eval::cmd_bench(&client, &dir, model.as_deref(), concurrency, &cwd)
                        .await
                        .map_err(|e| Error::custom(format!("eval bench: {e}")))?;
                }
            }
        }
        return Ok(());
    }

    // -- RPC mode: JSON-RPC over stdin/stdout for embedding CADE in other processes
    #[cfg(feature = "integration")]
    if args.mode.as_deref() == Some("rpc") {
        let session_opts = cade_sdk::session::SessionOptions {
            server_url: base_url.clone(),
            api_key: settings.api_key().unwrap_or_default(),
            agent_id: Some(agent.id.clone()),
            cwd: cwd.clone(),
            ..Default::default()
        };
        match cade_sdk::session::AgentSession::create(session_opts).await {
            Ok(sdk_session) => {
                cade_sdk::rpc::run_rpc_server(sdk_session).await;
                return Ok(());
            }
            Err(e) => return Err(Error::custom(format!("RPC session init: {e}"))),
        }
    }

    let headless_prompt: Option<String> = match (&args.prompt, &piped_stdin) {
        (Some(p), Some(stdin)) => Some(format!("{stdin}\n\n{p}")),
        (Some(p), None) => Some(p.clone()),
        (None, Some(stdin)) => Some(stdin.clone()),
        (None, None) => None,
    };

    if let Some(prompt) = headless_prompt {
        let fmt = args.effective_output_format();
        let timeout_secs = args.timeout_secs;

        // SessionStart hook (non-blocking) for headless runs
        if !hook_engine.is_empty() {
            hook_engine.session_start(&agent.id).await;
        }

        if fmt == "stream-json" {
            let run = cli::headless::run_headless_stream_json(
                &client,
                &agent.id,
                &default_model,
                &prompt,
                &permissions,
                &mcp,
                &hook_engine,
            );
            if timeout_secs > 0 {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), run).await
                {
                    Ok(_) => {}
                    Err(_) => {
                        eprintln!(
                            "{}",
                            json!({
                                "type":     "result",
                                "subtype":  "error",
                                "is_error": true,
                                "error":    format!("Headless run timed out after {timeout_secs}s"),
                                "agent_id": agent.id,
                            })
                        );
                        if !hook_engine.is_empty() {
                            hook_engine.session_end(&agent.id).await;
                        }
                        std::process::exit(124);
                    }
                }
            } else {
                run.await;
            }
            if !hook_engine.is_empty() {
                hook_engine.session_end(&agent.id).await;
            }
            std::process::exit(0);
        }

        let run = cli::headless::run_headless(
            &client,
            &agent.id,
            &prompt,
            &permissions,
            &mcp,
            &hook_engine,
            None,
        );
        let result = if timeout_secs > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), run).await {
                Ok(r) => r,
                Err(_) => {
                    if fmt == "json" {
                        eprintln!(
                            "{}",
                            json!({
                                "type":     "result",
                                "subtype":  "error",
                                "is_error": true,
                                "error":    format!("Headless run timed out after {timeout_secs}s"),
                                "agent_id": agent.id,
                            })
                        );
                    } else {
                        eprintln!("Error: headless run timed out after {timeout_secs}s");
                    }
                    if !hook_engine.is_empty() {
                        hook_engine.session_end(&agent.id).await;
                    }
                    std::process::exit(124);
                }
            }
        } else {
            run.await
        };

        if !hook_engine.is_empty() {
            hook_engine.session_end(&agent.id).await;
        }

        match result {
            Ok((output, stats)) => {
                if fmt == "json" {
                    println!(
                        "{}",
                        json!({
                            "type":        "result",
                            "subtype":     "success",
                            "is_error":    false,
                            "duration_ms": stats.duration_ms as u64,
                            "num_turns":   stats.turn_count,
                            "result":      output,
                            "agent_id":    agent.id,
                        })
                    );
                } else {
                    println!("{}", sanitize_for_terminal(&output));
                }
                std::process::exit(0);
            }
            Err(e) => {
                if fmt == "json" {
                    eprintln!(
                        "{}",
                        json!({
                            "type":    "result",
                            "subtype": "error",
                            "is_error": true,
                            "error":   e.to_string(),
                            "agent_id": agent.id,
                        })
                    );
                } else {
                    eprintln!("Error: {}", sanitize_for_terminal(&e.to_string()));
                }
                std::process::exit(1);
            }
        }
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
        // Show attached tools
        match client.get_agent_tools(&agent.id).await {
            Ok(tools) if !tools.is_empty() => {
                let names: Vec<&str> = tools.iter().map(|(_, n)| n.as_str()).collect();
                println!("Tools   : {} ({})", tools.len(), names.join(", "));
            }
            Ok(_) => println!("Tools   : 0 (none attached — use --link or /link)"),
            Err(_) => {}
        }
        return Ok(());
    }

    // Export agent
    if let Some(name_or_id) = &args.export_agent {
        let target_id = cli::export_import::resolve_agent_id(&client, name_or_id)
            .await
            .map_err(|e| Error::custom(format!("--export-agent: resolve agent: {e}")))?;
        let out_path = args
            .output
            .clone()
            .unwrap_or_else(|| cli::export_import::default_export_path(name_or_id));
        cli::export_import::export_agent_to_file(&client, &target_id, &out_path)
            .await
            .map_err(|e| Error::custom(format!("--export-agent: {e}")))?;
        return Ok(());
    }

    // Import agent
    if let Some(import_path) = &args.import_agent {
        let new_id = cli::export_import::import_agent_from_file(&client, import_path)
            .await
            .map_err(|e| Error::custom(format!("--import-agent: {e}")))?;
        println!("Agent ID: {new_id}");
        return Ok(());
    }

    // Load active theme before settings is moved into Arc
    let theme_colors = {
        use cade_core::resources::discover_themes;
        use cade_tui::ThemeColors;
        let theme_name = settings.global().theme.clone().unwrap_or_default();
        if theme_name.is_empty() || theme_name == "dark" {
            ThemeColors::dark()
        } else if theme_name == "light" {
            ThemeColors::light()
        } else {
            let discovered = discover_themes(&cwd, &agent_dir);
            discovered
                .iter()
                .find(|t| t.name == theme_name)
                .map(ThemeColors::from_theme)
                .unwrap_or_else(ThemeColors::dark)
        }
    };

    // Build execution backend from settings before moving settings into Arc (Phase 6)
    let exec_backend: std::sync::Arc<dyn cade_agent::backends::ExecutionBackend> = {
        let profile = settings.execution_profile().clone();
        let b = cade_agent::backends::backend_from_profile(&profile);
        let backend_name = b.name();
        if backend_name != "local" {
            eprintln!("  Execution backend: {backend_name}");
        }
        std::sync::Arc::from(b)
    };

    // Interactive REPL
    let settings_arc = Arc::new(Mutex::new(settings));
    let session_arc = Arc::new(Mutex::new(session));
    // Use the agent's actual model from DB as the initial REPL model.
    let initial_model = agent.model.clone().unwrap_or(default_model.to_string());

    let repl = Repl::new(
        client,
        agent.id,
        agent.name,
        permissions,
        initial_model,
        args.reasoning.clone(),
        settings_arc,
        session_arc,
        cwd.clone(),
        loaded_skills,
        skills_dir,
        toolset,
        hook_engine,
        conversation_id,
        mcp,
        theme_colors,
        exec_backend,
        capabilities,
    );
    // --continue: mark first turn as already done so env context isn't re-injected
    if args.continue_last {
        repl.mark_continued();
    }
    repl.run().await?;

    Ok(())
}

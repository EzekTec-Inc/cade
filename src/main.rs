// Re-use modules declared in lib.rs
use cade::agent;
use cade::cli;
use cade::desktop;
use cade::hooks::HookEngine;
use cade::mcp::McpManager;
use cade::permissions;
use cade::settings;
use cade::skills;
use cade::tools::schemas_for_names;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;

use std::sync::{Arc, Mutex};

use agent::{
    client::{CreateAgentRequest, MemoryBlock},
    session::SessionStore,
    tools::register_cade_tools,
    CadeClient,
};
use cade::toolsets::Toolset;
use cli::{Args, Repl};
use permissions::{PermissionManager, PermissionMode};
use settings::SettingsManager;
use skills::{discover_all_skills, skills_listing};

const SKILLS_DIR: &str = ".skills";

/// Base system prompt — behavioral instructions for the agent.
/// This is separate from the `persona` memory block (which holds identity/style).
/// The system prompt is instructions; memory blocks hold evolving state.
const BASE_SYSTEM_PROMPT: &str = "\
You are CADE (Coding AI assistant with Desktop Extensions), a stateful AI coding agent \
running in the user's terminal.\n\
\n\
## How you work\n\
\n\
Your tools run locally on the user's machine. Every Bash command, file read, and edit \
executes on their real filesystem. Be precise and careful.\n\
\n\
## Tool usage guidelines\n\
\n\
- **Explore before modifying**: Use Read/Glob/Grep to understand code before editing.\n\
- **Verify changes**: After editing, re-read the modified section to confirm correctness.\n\
- **Bash for builds/tests**: Always run the build/test after code changes to catch errors.\n\
- **update_memory**: When you learn something worth remembering — user preferences, \
project conventions, key facts — call update_memory immediately. Don't wait.\n\
- **Concise responses**: Lead with the answer or action. Skip preamble.\n\
- **No self-introduction**: Never introduce yourself or describe your capabilities unless \n\
  explicitly asked (e.g. \"who are you?\"). The user already knows who you are. \n\
  Start every response by directly addressing the task or question.\n\
\n\
## Memory\n\
\n\
Your memory blocks (injected below) persist across sessions. The `persona` block describes \
your identity. The `human` block holds facts about the user. The `project` block holds \
current project context. Update them proactively as you learn.\n\
";

/// Default memory block labels and their seed values.
/// (label, value, description)
const DEFAULT_MEMORY_BLOCKS: &[(&str, &str, &str)] = &[
    (
        "persona",
        "I prefer terse, accurate responses — I lead with action and skip preamble. \
         I explore before modifying, verify after editing, and update memory proactively. \
         I never introduce myself unprompted; I address the task directly.",
        "Who I am, what I value, and how I approach working with people",
    ),
    (
        "human",
        "",
        "What I know about the person I'm working with — their name, preferences, and working style",
    ),
    (
        "project",
        "",
        "Current project context, tech stack, conventions, and ongoing work",
    ),
];

/// Seed default memory blocks for a newly created agent.
async fn seed_default_memory(client: &CadeClient, agent_id: &str) {
    for (label, value, description) in DEFAULT_MEMORY_BLOCKS {
        if let Err(e) = client.upsert_memory(agent_id, label, value, Some(description)).await {
            tracing::warn!("seed_memory {label}: {e}");
        }
    }
}

/// Forward API keys from the CLI's environment to cade-server.
///
/// cade-server is a separate process and may not share the same environment.
/// This bridges the gap so that `export ANTHROPIC_API_KEY=...` in the user's
/// terminal is automatically propagated to the server.
async fn push_env_providers_to_server(client: &CadeClient) {
    // (name, kind, env_vars, base_url)
    let core: &[(&str, &str, &[&str], Option<&str>)] = &[
        ("anthropic", "anthropic", &["ANTHROPIC_API_KEY", "CLAUDE_API_KEY"], None),
        ("openai",    "openai",    &["OPENAI_API_KEY"],                       None),
        ("gemini",    "gemini",    &["GOOGLE_API_KEY", "GEMINI_API_KEY"],      None),
    ];
    for (name, kind, vars, base_url) in core {
        let key = vars.iter()
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()));
        if let Some(key) = key {
            let _ = client.add_provider(name, kind, Some(&key), *base_url).await;
        }
    }
    // Preset OpenAI-compatible providers (Groq, OpenRouter, Together, etc.)
    let presets: &[(&str, &[&str], &str)] = &[
        ("openrouter", &["OPENROUTER_API_KEY"],              "https://openrouter.ai/api/v1/chat/completions"),
        ("groq",       &["GROQ_API_KEY"],                    "https://api.groq.com/openai/v1/chat/completions"),
        ("together",   &["TOGETHER_API_KEY", "TOGETHER_AI_API_KEY"], "https://api.together.xyz/v1/chat/completions"),
        ("fireworks",  &["FIREWORKS_API_KEY"],               "https://api.fireworks.ai/inference/v1/chat/completions"),
        ("deepinfra",  &["DEEPINFRA_API_KEY"],               "https://api.deepinfra.com/v1/openai/chat/completions"),
    ];
    for (name, vars, base_url) in presets {
        let key = vars.iter()
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()));
        if let Some(key) = key {
            let _ = client.add_provider(name, "openai-compatible", Some(&key), Some(base_url)).await;
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

/// Register the `run_subagent` tool — spawns a focused subagent for a task.
async fn register_run_subagent_tool(client: &CadeClient) {
    let schema = serde_json::json!({
        "name": "run_subagent",
        "description": "Spawn a subagent to handle a task autonomously. Only the final answer \
    is returned — your context stays clean. Use for: codebase search (explore), implementation \
    (general-purpose, coder), code review (reviewer), or custom subagents.",
        "input_schema": {
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": "Built-in type (explore, general-purpose, coder, reviewer) or custom name from .cade/agents/"
                },
                "prompt": {
                    "type": "string",
                    "description": "The task description for the subagent"
                },
                "background": {
                    "type": "boolean",
                    "description": "Run in background — tool returns immediately, you get notified on completion (default false)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional: deploy an existing stateful agent as the subagent by its agent ID"
                },
                "model": {
                    "type": "string",
                    "description": "Optional: override the subagent's model"
                }
            },
            "required": ["subagent_type", "prompt"]
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
        tracing::debug!("run_subagent tool: {e}");
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
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this block is for (optional, shown in /memory display)"
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
    register_update_memory_tool(client).await;
    register_load_skill_tool(client).await;
    register_install_skill_tool(client).await;
    register_run_subagent_tool(client).await;
    let tools = register_cade_tools_filtered(client, toolset, tool_filter)
        .await
        .unwrap_or_default();
    let ids: Vec<String> = tools.iter().map(|t| t.id.clone()).collect();
    tracing::info!("Registered {} tools", tools.len());
    if !ids.is_empty() {
        if let Err(e) = client.attach_agent_tools(agent_id, &ids).await {
            tracing::warn!("attach_agent_tools: {e}");
        }
    }
}

async fn register_cade_tools_filtered(
    client: &CadeClient,
    toolset: Toolset,
    filter: Option<&[String]>,
) -> anyhow::Result<Vec<agent::client::ToolDef>> {
    // schemas_for_toolset and schemas_for_names imported at top-level
    // When no filter, use normal registration path
    if filter.is_none() {
        return register_cade_tools(client, toolset).await;
    }
    let names = filter.unwrap();
    let schemas = if names.is_empty() {
        // Empty filter → no tools (analysis-only mode)
        vec![]
    } else {
        schemas_for_names(toolset, names)
    };

    // Reuse the existing tool registration logic by passing schemas directly
    use agent::client::CreateToolRequest;
    use agent::tools::build_python_stub_from_schema as bps;
    let existing = client.list_tools().await.unwrap_or_default();
    let existing_map: std::collections::HashMap<String, agent::client::ToolDef> =
        existing.into_iter().map(|t| (t.name.clone(), t)).collect();

    let mut registered = Vec::new();
    for schema in schemas {
        let name        = schema["name"].as_str().unwrap_or("").to_string();
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
        // Auto-start cade-server from the same directory as the cade binary.
        // This allows the user to run a single `./cade` command without
        // needing a separate terminal for the server.
        let server_bin = std::env::current_exe()
            .ok()
            .map(|p| p.with_file_name("cade-server"))
            .filter(|p| p.exists());

        if let Some(server_bin) = server_bin {
            eprintln!("cade-server not running — starting…");
            let mut cmd = std::process::Command::new(&server_bin);
            // Log server output to /tmp/cade-server.log for debugging
            if let Ok(log) = std::fs::OpenOptions::new()
                .create(true).append(true).open("/tmp/cade-server.log")
            {
                use std::os::unix::io::IntoRawFd;
                let fd = log.into_raw_fd();
                // SAFETY: we own the fd; it is valid and open
                unsafe {
                    use std::os::unix::io::FromRawFd;
                    cmd.stdout(std::fs::File::from_raw_fd(fd));
                    cmd.stderr(std::fs::File::from_raw_fd(fd));
                }
            } else {
                cmd.stdout(std::process::Stdio::null())
                   .stderr(std::process::Stdio::null());
            }
            let _child = cmd.spawn().context("auto-start cade-server")?;

            // Poll up to 5 s for the server to become ready
            let mut ready = false;
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                if client.health().await.unwrap_or(false) {
                    ready = true;
                    break;
                }
            }
            if !ready {
                bail!(
                    "cade-server failed to start. Check /tmp/cade-server.log\n\
                     Or start it manually: {}", server_bin.display()
                );
            }
            eprintln!("cade-server ready.");
        } else {
            bail!(
                "Cannot connect to CADE server at {base_url}.\n\
                 Start cade-server first: ./target/release/cade-server"
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
        .context("invalid permission mode")?;
    let permissions = PermissionManager::new(perm_mode);

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
    let default_model = args
        .model
        .clone()
        .or_else(|| std::env::var("CADE_DEFAULT_MODEL").ok())
        .unwrap_or(server_info.2);

    // Detect toolset: --toolset flag > model family auto-detection
    let toolset = args
        .toolset
        .as_deref()
        .and_then(Toolset::from_str)
        .unwrap_or_else(|| Toolset::for_model(&default_model));
    tracing::info!(
        "Toolset: {} (model={})",
        toolset.display_name(),
        default_model
    );

    // Skills listing — compact (names + descriptions only), not full bodies.
    // The agent uses load_skill(id) to pull full content on-demand.
    let skills_block = skills_listing(&loaded_skills);

    // Agent resolution — helper closure avoids repeating the create logic
    let make_req = |model: String, desc: &str| {
        // Inject only the compact listing as a memory block.
        // Full skill content is loaded on-demand by the agent via load_skill tool.
        let memory_blocks: Vec<MemoryBlock> = if let Some(ctx) = &skills_block {
            vec![MemoryBlock {
                label: "skills".to_string(),
                value: ctx.clone(),
                description: None,
            }]
        } else {
            vec![]
        };
        CreateAgentRequest {
            name: Some(format!(
                "CADE-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            )),
            model,
            description: Some(desc.to_string()),
            system_prompt: Some(BASE_SYSTEM_PROMPT.to_string()),
            memory_blocks,
            tool_ids: vec![],
        }
    };

    let tool_filter: Option<Vec<String>> = args.tool_filter();

    let agent = if args.new_agent {
        let a = client
            .create_agent(make_req(
                default_model.clone(),
                "CADE coding agent with desktop extensions",
            ))
            .await
            .context("create agent")?;
        register_and_attach_filtered(&client, &a.id, toolset, tool_filter.as_deref()).await;
        seed_default_memory(&client, &a.id).await;
        session
            .set_agent(a.id.clone(), Some(a.name.clone()))
            .context("save session")?;
        settings
            .set_last_agent(&a.id)
            .context("save global session")?;
        a
    } else if let Some(id) = &args.agent {
        client
            .get_agent(id)
            .await
            .with_context(|| format!("get agent {id}"))?
    } else if let Some(name_query) = &args.name {
        // --name: find agent by name (partial, case-insensitive)
        let all = client.list_agents().await.context("list agents for --name")?;
        let q = name_query.to_lowercase();
        let matched: Vec<_> = all.iter().filter(|a| a.name.to_lowercase().contains(&q)).collect();
        match matched.len() {
            0 => anyhow::bail!("No agent matching --name '{name_query}'"),
            1 => client.get_agent(&matched[0].id).await
                    .with_context(|| format!("get agent {}", matched[0].id))?,
            n => anyhow::bail!(
                "{n} agents match '{name_query}': {}",
                matched.iter().map(|a| format!("{} ({})", a.name, a.id)).collect::<Vec<_>>().join(", ")
            ),
        }
    } else if let Some(last_id) = session.session.agent_id.clone() {
        match client.get_agent(&last_id).await {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Previous agent {last_id} not found — creating new agent");
                let a = client
                    .create_agent(make_req(default_model.clone(), "CADE coding agent"))
                    .await
                    .context("create agent")?;
                register_and_attach_filtered(&client, &a.id, toolset, tool_filter.as_deref()).await;
                seed_default_memory(&client, &a.id).await;
                session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                settings.set_last_agent(&a.id)?;
                a
            }
        }
    } else {
        println!("No previous session — creating new agent…");
        let a = client
            .create_agent(make_req(
                default_model.clone(),
                "CADE coding agent with desktop extensions",
            ))
            .await
            .context("create agent")?;
        register_and_attach_filtered(&client, &a.id, toolset, tool_filter.as_deref()).await;
        seed_default_memory(&client, &a.id).await;
        session.set_agent(a.id.clone(), Some(a.name.clone()))?;
        settings.set_last_agent(&a.id)?;
        a
    };

    // ── Conversation resolution ───────────────────────────────────────────────
    //
    // Precedence: --new (create new) > --resume (picker) > --continue (reuse saved) > saved session
    let conversation_id: Option<String> = if args.new_conversation {
        // Create a fresh conversation on the resolved agent
        match client.create_conversation(&agent.id, "").await {
            Ok(conv) => {
                let cid = conv["id"].as_str().unwrap_or("").to_string();
                session.set_conversation(Some(cid.clone())).context("save conversation")?;
                Some(cid)
            }
            Err(e) => {
                eprintln!("Warning: failed to create conversation: {e}");
                None
            }
        }
    } else if args.resume {
        // Interactive conversation picker (show before REPL starts)
        match client.list_conversations(&agent.id).await {
            Ok(convs) if !convs.is_empty() => {
                // Quick TTY picker: numbered list, pick by number
                println!("\nConversations for {}:", agent.name);
                for (i, c) in convs.iter().enumerate() {
                    let title = c["title"].as_str().unwrap_or("(untitled)");
                    let cnt   = c["message_count"].as_i64().unwrap_or(0);
                    println!("  [{}] {}  ({} msgs)", i + 1, title, cnt);
                }
                println!("  [n] Start new conversation");
                print!("\nChoice [1-{}]: ", convs.len());
                use std::io::Write;
                std::io::stdout().flush()?;
                let mut buf = String::new();
                std::io::stdin().read_line(&mut buf)?;
                let choice = buf.trim();
                if choice == "n" || choice == "N" {
                    let conv = client.create_conversation(&agent.id, "").await
                        .context("create conversation")?;
                    let cid = conv["id"].as_str().unwrap_or("").to_string();
                    session.set_conversation(Some(cid.clone())).context("save conversation")?;
                    Some(cid)
                } else if let Ok(n) = choice.parse::<usize>() {
                    if n >= 1 && n <= convs.len() {
                        let cid = convs[n - 1]["id"].as_str().unwrap_or("").to_string();
                        session.set_conversation(Some(cid.clone())).context("save conversation")?;
                        Some(cid)
                    } else {
                        eprintln!("Invalid choice — using default conversation");
                        None
                    }
                } else {
                    None
                }
            }
            Ok(_) => {
                println!("No conversations yet — starting new one");
                match client.create_conversation(&agent.id, "").await {
                    Ok(conv) => {
                        let cid = conv["id"].as_str().unwrap_or("").to_string();
                        session.set_conversation(Some(cid.clone())).context("save conversation")?;
                        Some(cid)
                    }
                    Err(e) => { eprintln!("Warning: {e}"); None }
                }
            }
            Err(e) => { eprintln!("Warning: list_conversations: {e}"); None }
        }
    } else {
        // Use saved conversation_id (--continue or resume from session)
        session.session.conversation_id.clone()
    };

    // ── MCP server startup ────────────────────────────────────────────────────
    let mcp_configs = settings.merged_mcp_servers();
    let mcp: std::sync::Arc<McpManager> = if mcp_configs.is_empty() {
        std::sync::Arc::new(McpManager::empty())
    } else {
        println!("Starting {} MCP server(s)…", mcp_configs.len());
        let mgr = McpManager::start(&mcp_configs).await;
        let count = mgr.status().len();
        let total  = mcp_configs.len();
        if count == 0 {
            eprintln!("Warning: no MCP servers started successfully");
        } else {
            println!("MCP: {count}/{total} server(s) ready");
        }
        std::sync::Arc::new(mgr)
    };

    // Register MCP tool schemas with cade-server + attach to agent
    if !mcp.is_empty() {
        use agent::tools::register_mcp_tools;
        let mcp_tool_ids: Vec<String> = register_mcp_tools(&client, mcp.all_tool_schemas())
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

    // Expose AGENT_ID to all child processes (bash tool, hooks, etc.)
    std::env::set_var("AGENT_ID", &agent.id);

    // --unlink: detach all tools from agent, then continue
    if args.unlink {
        match client.detach_agent_tools(&agent.id).await {
            Ok(n) => println!("✓ Detached {n} tool(s) from agent"),
            Err(e) => eprintln!("Warning: detach failed: {e}"),
        }
    }

    // --link: (re-)attach native + MCP tools to agent, then continue
    if args.link {
        register_and_attach(&client, &agent.id, toolset).await;
        if !mcp.is_empty() {
            use agent::tools::register_mcp_tools;
            let mcp_ids: Vec<String> = register_mcp_tools(&client, mcp.all_tool_schemas())
                .await.unwrap_or_default().into_iter().map(|t| t.id).collect();
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
            Err(e) => { eprintln!("✗ {e}"); std::process::exit(1); }
        }
        return Ok(());
    }

    // Migrate old system prompt: if the stored prompt is the minimal server fallback
    // (no "Never introduce yourself" rule) update it to BASE_SYSTEM_PROMPT.
    // This runs once per old agent; after the update the check is skipped.
    if agent.system_prompt.as_deref()
        .map(|p| !p.contains("Never introduce yourself"))
        .unwrap_or(true)
    {
        if let Err(e) = client.patch_agent_system_prompt(&agent.id, BASE_SYSTEM_PROMPT).await {
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
                    let (_, new_val, new_desc) = DEFAULT_MEMORY_BLOCKS[0]; // persona entry
                    let _ = client.upsert_memory(&agent.id, "persona", new_val, Some(new_desc)).await;
                    break;
                }
            }
        }
    }

    // Tray
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
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    };

    let headless_prompt: Option<String> = match (&args.prompt, &piped_stdin) {
        (Some(p), Some(stdin)) => Some(format!("{stdin}\n\n{p}")),
        (Some(p), None) => Some(p.clone()),
        (None, Some(stdin)) => Some(stdin.clone()),
        (None, None) => None,
    };

    if let Some(prompt) = headless_prompt {
        let fmt = args.effective_output_format();

        if fmt == "stream-json" {
            cli::headless::run_headless_stream_json(
                &client, &agent.id, &default_model, &prompt, &permissions, &mcp
            ).await;
            std::process::exit(0);
        }

        let result = cli::headless::run_headless(&client, &agent.id, &prompt, &permissions, &mcp).await;
        match result {
            Ok((output, stats)) => {
                if fmt == "json" {
                    println!("{}", serde_json::json!({
                        "type":        "result",
                        "subtype":     "success",
                        "is_error":    false,
                        "duration_ms": stats.duration_ms as u64,
                        "num_turns":   stats.turn_count,
                        "result":      output,
                        "agent_id":    agent.id,
                    }));
                } else {
                    println!("{output}");
                }
                std::process::exit(0);
            }
            Err(e) => {
                if fmt == "json" {
                    eprintln!("{}", serde_json::json!({
                        "type":    "result",
                        "subtype": "error",
                        "is_error": true,
                        "error":   e.to_string(),
                        "agent_id": agent.id,
                    }));
                } else {
                    eprintln!("Error: {e}");
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

    // Interactive REPL
    let settings_arc = Arc::new(Mutex::new(settings));
    let session_arc = Arc::new(Mutex::new(session));
    // Use the agent's actual model from DB as the initial REPL model.
    // default_model is the server-detected default for NEW agents;
    // for EXISTING agents the DB value is what the server actually uses for inference.
    let initial_model = agent.model.clone().unwrap_or(default_model.clone());

    let repl = Repl::new(
        client,
        agent.id,
        agent.name,
        permissions,
        initial_model,
        settings_arc,
        session_arc,
        cwd.clone(),
        loaded_skills,
        skills_dir,
        toolset,
        hook_engine,
        conversation_id,
        mcp,
    );
    // --continue: mark first turn as already done so env context isn't re-injected
    if args.continue_last {
        repl.mark_continued();
    }
    repl.run().await?;

    Ok(())
}

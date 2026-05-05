use crate::bootstrap::memory::seed_default_memory;
use crate::bootstrap::prompt::build_system_prompt;
use crate::bootstrap::tools::register_and_attach_with_caps_filtered;
use cade::cli::Args;
use cade::settings::SettingsManager;
use cade::skills::Skill;
use cade::skills::{discover_all_skills, skills_listing};
use cade::toolsets::Toolset;
use cade::{Error, Result};
use cade_agent::agent;

/// Snapshot the agent's current `active_goal` value into archival memory and
/// then clear it.  Used on `--new` / picker "n" paths so accidentally
/// starting a fresh conversation does not silently lose the previous plan.
///
/// All errors are best-effort logged: the original /new behaviour (delete and
/// proceed) is preserved on any failure so the user is never blocked from
/// starting a new conversation.
async fn archive_and_clear_active_goal(
    client: &agent::HttpTransport,
    agent_id: &str,
    conversation_id: Option<&str>,
) {
    let prev = client
        .get_memory(agent_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|b| b.label == "active_goal")
        .map(|b| b.value);
    if let Some(value) = prev {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let conv_part = conversation_id
                .map(|c| format!(" (conversation {})", &c[..c.len().min(20)]))
                .unwrap_or_default();
            let snapshot = format!(
                "[active_goal snapshot — archived on bootstrap --new{conv_part}]\n\n{trimmed}"
            );
            let tags = vec![
                "active_goal".to_string(),
                "snapshot".to_string(),
                "bootstrap_new".to_string(),
            ];
            let _ = client
                .insert_archival_memory(agent_id, &snapshot, &tags)
                .await;
        }
    }
    let _ = client.delete_memory(agent_id, "active_goal").await;
}
use cade_agent::agent::HttpTransport;
use cade_agent::agent::client::{CreateAgentRequest, MemoryBlock};
use cade_agent::agent::session::SessionStore;
use cade_core::capabilities::CapabilitySet;

pub async fn resolve_agent_and_conversation(
    client: &HttpTransport,
    args: &Args,
    default_model: &str,
    toolset: Toolset,
    skills_block: &Option<String>,
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
    session: &mut SessionStore,
    settings: &mut SettingsManager,
    capabilities: &CapabilitySet,
) -> Result<(
    agent::client::AgentState,
    Vec<Skill>,
    Option<String>,
    String,
)> {
    // Build system prompt: base + any context files (AGENTS.md, CLAUDE.md, CADE.md)
    let context_files = cade_core::resources::context_files::discover_context_files(cwd, agent_dir);
    let context_block = cade_core::resources::context_files::build_context_block(&context_files);
    let base_prompt = build_system_prompt(capabilities);
    let effective_system_prompt = if context_block.is_empty() {
        base_prompt
    } else {
        format!(
            "{base_prompt}, Consider the following context if required/relevant to what you're working on:{context_block}"
        )
    };
    if !context_files.is_empty() {
        let names: Vec<String> = context_files
            .iter()
            .map(|f| {
                f.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect();
        tracing::info!(
            "Loaded {} context file(s): {}",
            context_files.len(),
            names.join(", ")
        );
    }

    let make_req = |model: String, desc: &str| {
        // Inject only the compact listing as a memory block.
        // Full skill content is loaded on-demand by the agent via load_skill tool.
        let memory_blocks: Vec<MemoryBlock> = if let Some(ctx) = &skills_block {
            vec![MemoryBlock {
                label: "skills".to_string(),
                value: ctx.clone(),
                description: None,
                tier: None,
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
            system_prompt: Some(effective_system_prompt.clone()),
            memory_blocks,
            tool_ids: vec![],
        }
    };

    let tool_filter: Option<Vec<String>> = args.tool_filter();

    let agent = if args.new_agent {
        let a = client
            .create_agent(make_req(
                default_model.to_string(),
                "CADE coding agent with desktop extensions",
            ))
            .await
            .map_err(|e| Error::custom(format!("create agent: {e}")))?;
        register_and_attach_with_caps_filtered(
            client,
            &a.id,
            toolset,
            capabilities,
            tool_filter.as_deref(),
        )
        .await;
        seed_default_memory(client, &a.id).await;
        session
            .set_agent(a.id.clone(), Some(a.name.clone()))
            .map_err(|e| Error::custom(format!("save session: {e}")))?;
        settings
            .set_last_agent(&a.id)
            .map_err(|e| Error::custom(format!("save global session: {e}")))?;
        a
    } else if let Some(id) = &args.agent {
        let a = client
            .get_agent(id)
            .await
            .map_err(|e| Error::custom(format!("get agent {id}: {e}")))?;
        session.set_agent(a.id.clone(), Some(a.name.clone()))?;
        settings.set_last_agent(&a.id)?;
        a
    } else if let Some(name_query) = &args.name {
        // --name: find agent by name (partial, case-insensitive)
        let all = client
            .list_agents()
            .await
            .map_err(|e| Error::custom(format!("list agents for --name: {e}")))?;
        let q = name_query.to_lowercase();
        let matched: Vec<_> = all
            .iter()
            .filter(|a| a.name.to_lowercase().contains(&q))
            .collect();
        let a = match matched.len() {
            0 => {
                return Err(Error::custom(format!(
                    "No agent matching --name '{name_query}'"
                )));
            }
            1 => client
                .get_agent(&matched[0].id)
                .await
                .map_err(|e| Error::custom(format!("get agent {}: {e}", matched[0].id)))?,
            n => {
                return Err(Error::custom(format!(
                    "{n} agents match '{name_query}': {}",
                    matched
                        .iter()
                        .map(|a| format!("{} ({})", a.name, a.id))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        session.set_agent(a.id.clone(), Some(a.name.clone()))?;
        settings.set_last_agent(&a.id)?;
        a
    } else if let Some(local_id) = session.session.agent_id.clone() {
        match client.get_agent(&local_id).await {
            Ok(a) => {
                // Cross-sync: local session agent → global last_agent
                let _ = settings.set_last_agent(&a.id);
                a
            }
            Err(_) => {
                eprintln!("Local project agent {local_id} not found — falling back");
                if let Some(last_id) = settings.last_agent().map(|s| s.to_string()) {
                    match client.get_agent(&last_id).await {
                        Ok(a) => a,
                        Err(_) => {
                            eprintln!(
                                "Previous global agent {last_id} not found — creating new agent"
                            );
                            let a = client
                                .create_agent(make_req(
                                    default_model.to_string(),
                                    "CADE coding agent",
                                ))
                                .await
                                .map_err(|e| Error::custom(format!("create agent: {e}")))?;
                            register_and_attach_with_caps_filtered(
                                client,
                                &a.id,
                                toolset,
                                capabilities,
                                tool_filter.as_deref(),
                            )
                            .await;
                            seed_default_memory(client, &a.id).await;
                            session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                            settings.set_last_agent(&a.id)?;
                            a
                        }
                    }
                } else {
                    println!("No previous session — creating new agent…");
                    let a = client
                        .create_agent(make_req(
                            default_model.to_string(),
                            "CADE coding agent with desktop extensions",
                        ))
                        .await
                        .map_err(|e| Error::custom(format!("create agent: {e}")))?;
                    register_and_attach_with_caps_filtered(
                        client,
                        &a.id,
                        toolset,
                        capabilities,
                        tool_filter.as_deref(),
                    )
                    .await;
                    seed_default_memory(client, &a.id).await;
                    session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                    settings.set_last_agent(&a.id)?;
                    a
                }
            }
        }
    } else if let Some(last_id) = settings.last_agent().map(|s| s.to_string()) {
        match client.get_agent(&last_id).await {
            Ok(a) => {
                // Cross-sync: global last_agent → local session
                let _ = session.set_agent(a.id.clone(), Some(a.name.clone()));
                a
            }
            Err(_) => {
                eprintln!("Previous agent {last_id} not found — creating new agent");
                let a = client
                    .create_agent(make_req(default_model.to_string(), "CADE coding agent"))
                    .await
                    .map_err(|e| Error::custom(format!("create agent: {e}")))?;
                register_and_attach_with_caps_filtered(
                    client,
                    &a.id,
                    toolset,
                    capabilities,
                    tool_filter.as_deref(),
                )
                .await;
                seed_default_memory(client, &a.id).await;
                session.set_agent(a.id.clone(), Some(a.name.clone()))?;
                settings.set_last_agent(&a.id)?;
                a
            }
        }
    } else {
        println!("No previous session — creating new agent…");
        let a = client
            .create_agent(make_req(
                default_model.to_string(),
                "CADE coding agent with desktop extensions",
            ))
            .await
            .map_err(|e| Error::custom(format!("create agent: {e}")))?;
        register_and_attach_with_caps_filtered(
            client,
            &a.id,
            toolset,
            capabilities,
            tool_filter.as_deref(),
        )
        .await;
        seed_default_memory(client, &a.id).await;
        session.set_agent(a.id.clone(), Some(a.name.clone()))?;
        settings.set_last_agent(&a.id)?;
        a
    };

    let loaded_skills = discover_all_skills(cwd, Some(&agent.id), None);
    if !loaded_skills.is_empty() {
        println!("Loaded {} skill(s)", loaded_skills.len());
    }
    let updated_skills_block = skills_listing(&loaded_skills);
    let _ = client
        .upsert_memory(
            &agent.id,
            "skills",
            updated_skills_block.as_deref().unwrap_or(""),
            None,
        )
        .await;

    // -- Auto-load required skills from `[project]` block
    //
    // Parse the project memory block for a "## Required Skills" section and
    // auto-load each listed skill so the agent has them in context from the
    // start, without relying on the LLM to remember to call `load_skill`.
    {
        let project_block = client
            .get_memory(&agent.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == "project")
            .map(|b| b.value)
            .unwrap_or_default();

        let required_skills = parse_required_skills(&project_block);
        if !required_skills.is_empty() {
            let mut auto_loaded = Vec::new();
            for skill_id in &required_skills {
                if let Some(skill) = loaded_skills.iter().find(|s| s.id == *skill_id) {
                    // Load the skill body into a memory block so it's in context
                    let skill_label = format!("skill:{skill_id}");
                    let skill_content = match std::fs::read_to_string(&skill.path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let _ = client
                        .upsert_memory(&agent.id, &skill_label, &skill_content, None)
                        .await;
                    auto_loaded.push(skill_id.as_str());
                }
            }
            if !auto_loaded.is_empty() {
                tracing::info!(
                    "Auto-loaded {} required skill(s) from [project] block: {}",
                    auto_loaded.len(),
                    auto_loaded.join(", ")
                );
            }
        }
    }

    // -- Conversation resolution
    //
    // Precedence: --new (create new) > --resume (picker) > --continue (reuse saved) > saved session
    let conversation_id: Option<String> = if args.new_conversation {
        // Create a fresh conversation on the resolved agent
        match client.create_conversation(&agent.id, "").await {
            Ok(conv) => {
                let cid = conv["id"].as_str().unwrap_or("").to_string();
                // Archive the previous active_goal then clear it so the agent forgets
                // the previous task without losing the trail.
                archive_and_clear_active_goal(client, &agent.id, Some(&cid)).await;
                session
                    .set_conversation(Some(cid.clone()))
                    .map_err(|e| Error::custom(format!("save conversation: {e}")))?;
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
                    let cnt = c["message_count"].as_i64().unwrap_or(0);
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
                    let conv = client
                        .create_conversation(&agent.id, "")
                        .await
                        .map_err(|e| Error::custom(format!("create conversation: {e}")))?;
                    let cid = conv["id"].as_str().unwrap_or("").to_string();
                    // Archive the previous active_goal then clear it so the agent forgets
                    // the previous task without losing the trail.
                    archive_and_clear_active_goal(client, &agent.id, Some(&cid)).await;
                    session
                        .set_conversation(Some(cid.clone()))
                        .map_err(|e| Error::custom(format!("save conversation: {e}")))?;
                    Some(cid)
                } else if let Ok(n) = choice.parse::<usize>() {
                    if n >= 1 && n <= convs.len() {
                        let cid = convs[n - 1]["id"].as_str().unwrap_or("").to_string();
                        session
                            .set_conversation(Some(cid.clone()))
                            .map_err(|e| Error::custom(format!("save conversation: {e}")))?;
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
                        session
                            .set_conversation(Some(cid.clone()))
                            .map_err(|e| Error::custom(format!("save conversation: {e}")))?;
                        Some(cid)
                    }
                    Err(e) => {
                        eprintln!("Warning: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: list_conversations: {e}");
                None
            }
        }
    } else {
        // Use saved conversation_id (--continue or resume from session)
        session.session.conversation_id.clone()
    };
    Ok((
        agent,
        loaded_skills,
        conversation_id,
        effective_system_prompt,
    ))
}

/// Parse a `[project]` memory block for a "## Required Skills" section.
///
/// Expects lines like `- skill-id` or `- skill-id [scope]` under the heading.
/// Returns the list of skill IDs found.
fn parse_required_skills(project_block: &str) -> Vec<String> {
    let mut in_section = false;
    let mut skills = Vec::new();
    for line in project_block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Required Skills") || trimmed.starts_with("## Required skills") {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with("## ") {
            // Hit the next section heading — stop parsing.
            break;
        }
        if in_section && trimmed.starts_with("- ") {
            // Extract skill ID: strip leading "- ", take first word.
            let rest = trimmed.trim_start_matches("- ").trim();
            let id = rest.split_whitespace().next().unwrap_or("").to_string();
            if !id.is_empty() {
                skills.push(id);
            }
        }
    }
    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_required_skills_extracts_ids() {
        let block = "\
# CADE Project Rules

## Allowed MCP Servers
- context7
- deepwiki

## Required Skills
Load and follow these skills for all work:
- tdd-guide
- strict-project-execution
- caveman
- grill-me
- rust

## Workflow Requirements
- Always index the workspace
";
        let skills = parse_required_skills(block);
        assert_eq!(
            skills,
            vec!["tdd-guide", "strict-project-execution", "caveman", "grill-me", "rust"]
        );
    }

    #[test]
    fn parse_required_skills_empty_block() {
        assert!(parse_required_skills("").is_empty());
    }

    #[test]
    fn parse_required_skills_no_section() {
        let block = "Some random project notes\n- not a skill";
        assert!(parse_required_skills(block).is_empty());
    }

    #[test]
    fn parse_required_skills_stops_at_next_heading() {
        let block = "\
## Required Skills
- alpha
- beta

## Something Else
- gamma
";
        let skills = parse_required_skills(block);
        assert_eq!(skills, vec!["alpha", "beta"]);
    }
}

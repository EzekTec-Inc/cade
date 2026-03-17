import re

with open("src/main.rs", "r") as f:
    content = f.read()

agent_res_match = re.search(r"    // Agent resolution — helper closure avoids repeating the create logic.*?\n(    let make_req.*?updated_skills_block\.as_deref\(\)\.unwrap_or\(\"\"\),\n.*?None,\n.*?\)\n.*?\.await;\n)", content, re.DOTALL)

conv_res_match = re.search(r"(    // ── Conversation resolution ───────────────────────────────────────────────.*?session\.session\.conversation_id\.clone\(\)\n    \};)\n", content, re.DOTALL)

if agent_res_match and conv_res_match:
    agent_code = agent_res_match.group(1)
    conv_code = conv_res_match.group(1)
    
    # We will replace these blocks with a function call
    call_code = """    let (agent, loaded_skills, conversation_id) = resolve_agent_and_conversation(
        &client,
        &args,
        &default_model,
        toolset,
        &skills_block,
        &cwd,
        &mut session,
        &mut settings,
    ).await?;
"""
    
    new_content = content.replace(agent_code, call_code)
    new_content = new_content.replace(conv_code + "\n", "")
    
    # Add the function definition
    func_code = """
async fn resolve_agent_and_conversation(
    client: &CadeClient,
    args: &Args,
    default_model: &str,
    toolset: Toolset,
    skills_block: &Option<String>,
    cwd: &std::path::Path,
    session: &mut SessionStore,
    settings: &mut SettingsManager,
) -> Result<(agent::client::AgentState, Vec<Skill>, Option<String>)> {
""" + agent_code + "\n" + conv_code + """
    Ok((agent, loaded_skills, conversation_id))
}
"""
    
    # Insert function before auto_start_server
    new_content = new_content.replace("async fn auto_start_server", func_code + "\nasync fn auto_start_server")
    
    with open("src/main.rs", "w") as f:
        f.write(new_content)
    print("Success")
else:
    print("Failed to match")

import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # Specifically replace agent_metrics.read().await and .write().await
    content = content.replace("state.agent_metrics.read().await", "state.agent_metrics")
    content = content.replace("state.agent_metrics.write().await", "state.agent_metrics")
    content = content.replace("state2.agent_metrics.read().await", "state2.agent_metrics.clone()")
    content = content.replace("state2.agent_metrics.write().await", "state2.agent_metrics.clone()")
    content = content.replace("agent_metrics.write().await", "agent_metrics")
    
    # Fix metric increments
    content = re.sub(r'([a-zA-Z0-9_]+)\.inflation_guard_hits \+= 1;', r'\1.inflation_guard_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);', content)
    content = re.sub(r'([a-zA-Z0-9_]+)\.tool_outputs_compacted \+= ([^;]+);', r'\1.tool_outputs_compacted.fetch_add(\2, std::sync::atomic::Ordering::Relaxed);', content)
    content = re.sub(r'([a-zA-Z0-9_]+)\.consolidation_runs \+= 1;', r'\1.consolidation_runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);', content)
    content = re.sub(r'([a-zA-Z0-9_]+)\.chars_summarised \+= ([^;]+);', r'\1.chars_summarised.fetch_add(\2, std::sync::atomic::Ordering::Relaxed);', content)
    content = re.sub(r'([a-zA-Z0-9_]+)\.chars_produced \+= ([^;]+);', r'\1.chars_produced.fetch_add(\2, std::sync::atomic::Ordering::Relaxed);', content)

    # Fix the .cloned() in agents.rs
    content = content.replace("let m = metrics.get(&agent_id).cloned().unwrap_or_default();\n    Ok(Json(json!(m)))", 
    "let json_val = metrics.get(&agent_id).map(|v| serde_json::json!(v.value())).unwrap_or_else(|| serde_json::json!(crate::server::state::AgentMetrics::default()));\n    Ok(Json(json_val))")

    with open(path, 'w') as f:
        f.write(content)

for p in [
    "crates/cade-server/src/server/api/agents.rs",
    "crates/cade-server/src/server/api/messages/context.rs",
    "crates/cade-server/src/server/api/messages/mod.rs",
    "crates/cade-server/src/server/api/run/mod.rs",
    "crates/cade-server/src/server/consolidation.rs"
]:
    fix_file(p)

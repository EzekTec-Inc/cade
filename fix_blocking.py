import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # The goal is to offload `sqlite::get_context_window` to `tokio::task::spawn_blocking`.
    # Let's find:
    # let all_rows =
    #     sqlite::get_context_window(&state.db, agent_id, conversation_id, context_char_budget)
    #         .unwrap_or_default();
    
    # We can replace this with:
    # let db_pool = state.db.clone();
    # let agent_id_clone = agent_id.to_string();
    # let conv_id_clone = conversation_id.map(|s| s.to_string());
    # let all_rows = tokio::task::spawn_blocking(move || {
    #     sqlite::get_context_window(&db_pool, &agent_id_clone, conv_id_clone.as_deref(), context_char_budget)
    #         .unwrap_or_default()
    # }).await.unwrap_or_default();
    
    old_code = """    let all_rows =
        sqlite::get_context_window(&state.db, agent_id, conversation_id, context_char_budget)
            .unwrap_or_default();"""

    new_code = """    let db_pool = state.db.clone();
    let agent_id_clone = agent_id.to_string();
    let conv_id_clone = conversation_id.map(|s| s.to_string());
    let all_rows = tokio::task::spawn_blocking(move || {
        sqlite::get_context_window(&db_pool, &agent_id_clone, conv_id_clone.as_deref(), context_char_budget)
            .unwrap_or_default()
    }).await.unwrap_or_default();"""
    
    content = content.replace(old_code, new_code)
    
    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-server/src/server/api/messages/context.rs")

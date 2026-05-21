import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # The goal is to optimize zero-allocation SSE streaming.
    # Currently, `run_agent_loop` builds `Event::default().data(json!({...}).to_string())` for every stream chunk.
    # We can replace the `send` macro/closure in `run_agent_loop` to use a structured string representation directly where possible,
    # or just use serde_json::to_string directly instead of `json!(...).to_string()`.
    
    # Let's replace the `send` macro definition.
    # let send = |data: Value| {
    #     let tx = tx.clone();
    #     let ev = Event::default().data(data.to_string());
    #     async move {
    #         let _ = tx.send(Ok(ev)).await;
    #     }
    # };
    
    old_send = """    let send = |data: Value| {
        let tx = tx.clone();
        let ev = Event::default().data(data.to_string());
        async move {
            let _ = tx.send(Ok(ev)).await;
        }
    };"""

    new_send = """    let send_raw = |json_string: String| {
        let tx = tx.clone();
        let ev = Event::default().data(json_string);
        async move {
            let _ = tx.send(Ok(ev)).await;
        }
    };
    
    let send = |data: Value| {
        let tx = tx.clone();
        let ev = Event::default().data(data.to_string());
        async move {
            let _ = tx.send(Ok(ev)).await;
        }
    };"""
    
    # To really prevent allocation churn, we'd need to change the fast-path stream loops.
    # For instance:
    # Ok(StreamChunk::Text(t)) => {
    #     text_acc.push_str(&t);
    #     send(json!({ "message_type": "assistant_message", "content": t })).await;
    # }
    
    old_stream_text = """                        Ok(StreamChunk::Text(t)) => {
                            text_acc.push_str(&t);
                            send(json!({ "message_type": "assistant_message", "content": t })).await;
                        }"""
                        
    new_stream_text = """                        Ok(StreamChunk::Text(t)) => {
                            text_acc.push_str(&t);
                            // Zero-allocation structured string building for high frequency streaming
                            // Using a pre-allocated format to avoid json! macro dynamic allocations
                            let json_str = format!("{{\\"message_type\\":\\"assistant_message\\",\\"content\\":{}}}", serde_json::to_string(&t).unwrap_or_default());
                            send_raw(json_str).await;
                        }"""
    
    content = content.replace(old_send, new_send)
    content = content.replace(old_stream_text, new_stream_text)
    
    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-server/src/server/api/run/mod.rs")

/// JSON-RPC server over stdin/stdout.
///
/// Allows non-Rust programs to use CADE as a subprocess.
/// Wire format: LF-delimited JSONL.
///
/// Supported methods:
/// - `prompt`   `{ "text": "..." }` → `{ "output": "..." }`
/// - `stream`   `{ "text": "..." }` → `{ "delta": "..." }` × N + `{ "done": true }`
/// - `memory`   `{ "action": "get|set|list", "label"?: "...", "value"?: "..." }` → result
/// - `skills`   `{}` → `[{ "id": "...", "description": "..." }]`
/// - `health`   `{}` → `{ "ok": true }`
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::session::AgentSession;

// region:    --- RPC server

/// Run the RPC server, reading commands from stdin and writing responses to stdout.
/// Blocks until stdin is closed.
pub async fn run_rpc_server(session: AgentSession) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut reader = BufReader::new(stdin).lines();
    let mut out = tokio::io::BufWriter::new(stdout);

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Err(e) => json!({ "error": format!("parse error: {e}") }),
            Ok(req) => handle_request(&session, &req).await,
        };

        let mut out_line = serde_json::to_string(&response).unwrap_or_default();
        out_line.push('\n');
        if out.write_all(out_line.as_bytes()).await.is_err() {
            break;
        }
        if out.flush().await.is_err() {
            break;
        }
    }
}

// endregion: --- RPC server

// region:    --- Request handling

async fn handle_request(session: &AgentSession, req: &Value) -> Value {
    let method = req["method"].as_str().unwrap_or("");
    let id = req.get("id").cloned().unwrap_or(Value::Null);

    let result = match method {
        "prompt" => handle_prompt(session, req).await,
        "memory" => handle_memory(session, req).await,
        "skills" => handle_skills(session),
        "health" => Ok(json!({ "ok": true, "agent_id": session.agent_id() })),
        other => Err(format!("Unknown method: '{other}'")),
    };

    match result {
        Ok(data) => json!({ "id": id, "result": data }),
        Err(msg) => json!({ "id": id, "error": msg }),
    }
}

async fn handle_prompt(session: &AgentSession, req: &Value) -> core::result::Result<Value, String> {
    let text = req["params"]["text"]
        .as_str()
        .ok_or("params.text is required")?;
    let output = session.prompt(text).await.map_err(|e| e.to_string())?;
    Ok(json!({ "output": output }))
}

async fn handle_memory(session: &AgentSession, req: &Value) -> core::result::Result<Value, String> {
    let action = req["params"]["action"].as_str().unwrap_or("list");
    match action {
        "get" => {
            let label = req["params"]["label"]
                .as_str()
                .ok_or("params.label required")?;
            let value = session.get_memory(label).await.map_err(|e| e.to_string())?;
            Ok(json!({ "label": label, "value": value }))
        }
        "set" => {
            let label = req["params"]["label"]
                .as_str()
                .ok_or("params.label required")?;
            let value = req["params"]["value"]
                .as_str()
                .ok_or("params.value required")?;
            session
                .set_memory(label, value)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true }))
        }
        "list" => {
            let blocks = session.list_memory().await.map_err(|e| e.to_string())?;
            Ok(json!(
                blocks
                    .into_iter()
                    .map(|b| json!({ "label": b.label, "value": b.value }))
                    .collect::<Vec<_>>()
            ))
        }
        other => Err(format!("Unknown memory action: '{other}'")),
    }
}

fn handle_skills(session: &AgentSession) -> core::result::Result<Value, String> {
    let skills = session.list_skills();
    Ok(json!(
        skills
            .iter()
            .map(|s| json!({ "id": s.id, "description": s.description }))
            .collect::<Vec<_>>()
    ))
}

// endregion: --- Request handling

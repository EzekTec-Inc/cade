use cade_mcp::config::McpServerConfig;
use cade_mcp::McpManager;
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    let mut configs = HashMap::new();
    configs.insert("mock-mcp-ui".to_string(), McpServerConfig {
        command: "python3".to_string(),
        args: vec!["/home/engr-uba/Downloads/02 Rust-project/CADE/mock_mcp.py".to_string()],
        env: Default::default(),
        core_server: false,
        disabled: false,
        write_tools: vec![],
    });

    let (manager, results) = McpManager::start(&configs).await;
    for res in results {
        println!("Result: {:?}", res);
    }
    let schemas = manager.all_tool_schemas().await;
    println!("Schemas: {}", serde_json::to_string_pretty(&schemas).unwrap());
}

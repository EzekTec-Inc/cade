use serde_json::{json, Value};

fn main() {
    let system = vec![
        "Hello".to_string(),
        "World".to_string(),
    ];
    let mut system_blocks: Vec<Value> = Vec::new();
    for m in system.iter() {
        system_blocks.push(json!({
            "type": "text",
            "text": m,
        }));
    }
    
    if let Some(first) = system_blocks.first_mut() {
        first["cache_control"] = json!({ "type": "ephemeral" });
    }
    
    println!("{}", serde_json::to_string_pretty(&system_blocks).unwrap());
}

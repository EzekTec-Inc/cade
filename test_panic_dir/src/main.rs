fn main() {
    let v: serde_json::Value = serde_json::json!({
        "choices": []
    });
    println!("Testing indexing...");
    let val = &v["choices"][0]["delta"];
    println!("Value: {:?}", val);
}

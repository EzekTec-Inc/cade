fn main() {
    let v: serde_json::Value = serde_json::json!({
        "choices": []
    });
    let _ = &v["choices"][0]["delta"];
}

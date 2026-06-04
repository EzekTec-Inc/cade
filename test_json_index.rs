fn main() {
    let v: serde_json::Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
    let delta = &v["choices"][0]["delta"];
    println!("delta: {:?}", delta);
}

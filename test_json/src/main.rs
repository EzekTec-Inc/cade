fn main() {
    let v: serde_json::Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
    println!("Does it panic?");
    let delta = &v["choices"][0];
    println!("delta: {:?}", delta);
}

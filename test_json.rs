use serde_json::json;

fn main() {
    let v = json!({
        "choices": []
    });
    println!("{:?}", v["choices"][0]);
}

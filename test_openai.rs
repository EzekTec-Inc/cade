use serde_json::Value;

fn main() {
    let data = r#"{"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}]}"#;
    let v: Value = serde_json::from_str(data).unwrap();
    let delta = &v["choices"][0]["delta"];
    println!("Content: {:?}", delta["content"].as_str());
    println!("Reasoning: {:?}", delta["reasoning"].as_str());
    println!("Tool calls: {:?}", delta["tool_calls"].as_array());
    println!("Finish reason: {:?}", v["choices"][0]["finish_reason"].as_str());
}

import re

with open("crates/cade-ai/src/openai.rs", "r") as f:
    content = f.read()

# Insert is_o_series function
content = content.replace(
    "fn needs_responses_api(model: &str) -> bool {",
    """fn is_o_series(model: &str) -> bool {
    let bare = bare_model(model).to_lowercase();
    bare.starts_with("o1") || bare.starts_with("o3") || bare.starts_with("o4")
}

fn needs_responses_api(model: &str) -> bool {"""
)

# Replace local is_o_series declarations
content = re.sub(
    r"let is_o_series = \{\s*let bare = bare_model\(&req\.model\)\.to_lowercase\(\);\s*bare\.starts_with\(\"o1\"\) \|\| bare\.starts_with\(\"o3\"\) \|\| bare\.starts_with\(\"o4\"\)\s*\};",
    "let is_o_series = is_o_series(&req.model);",
    content
)

# Replace reasoning_effort checks
content = content.replace(
    "if let Some(effort) = &req.reasoning_effort {",
    "if is_o_series(&req.model) && let Some(effort) = &req.reasoning_effort {"
)

with open("crates/cade-ai/src/openai.rs", "w") as f:
    f.write(content)

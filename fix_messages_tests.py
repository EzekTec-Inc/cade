import re

with open("crates/cade-server/src/server/api/messages/tests.rs", "r") as f:
    content = f.read()

# Replace max_tokens_per_turn
content = re.sub(r'max_tokens_per_turn:\s*64_000', 'max_tokens_per_turn: Some(64_000)', content)

# Fix missing argument in group_into_turns
if "fn group_into_turns_helper(" not in content:
    # Rename function call
    content = content.replace("group_into_turns(", "group_into_turns_helper(")
    # We also need to add the helper, but since we replaced ALL instances,
    # if there was a `fn group_into_turns(` we also renamed it to `fn group_into_turns_helper(`.
    # Luckily, `fn group_into_turns` is in `context.rs`, not in `tests.rs`! The tests just use it.
    
    helper = """
fn group_into_turns_helper(messages: &[LlmMessage]) -> Vec<Vec<LlmMessage>> {
    group_into_turns(messages, 8000)
}
"""
    # Insert helper after the use statements
    content = content.replace("use super::*;", "use super::*;\n" + helper)

with open("crates/cade-server/src/server/api/messages/tests.rs", "w") as f:
    f.write(content)

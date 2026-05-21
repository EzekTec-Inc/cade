import re
import json

with open('crates/cade-server/src/server/api/mod.rs', 'r') as f:
    content = f.read()

start_idx = content.find('pub fn router(state: AppState) -> Router {')
end_idx = content.find('    Router::new()', start_idx)

if start_idx != -1 and end_idx != -1:
    old_text = content[start_idx:end_idx]
    
    # Only replace :param when it follows a slash
    new_text = re.sub(r'/:([a-zA-Z_]+)', r'/{\1}', old_text)
    
    with open('edit_payload.json', 'w') as f:
        json.dump({
            "path": "crates/cade-server/src/server/api/mod.rs",
            "edits": [{"oldText": old_text, "newText": new_text}]
        }, f)

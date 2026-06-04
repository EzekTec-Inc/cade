import sys
import re

with open('crates/cade-server/src/server/api/messages/mod.rs', 'r') as f:
    code = f.read()

code = code.replace(
    'Event::default().data(json!({ "error": err_msg }).to_string())',
    'Event::default().data(json!({ "message_type": "error", "error": err_msg }).to_string())'
)

code = code.replace(
    'Event::default().data(json!({ "error": e.to_string() }).to_string())',
    'Event::default().data(json!({ "message_type": "error", "error": e.to_string() }).to_string())'
)

with open('crates/cade-server/src/server/api/messages/mod.rs', 'w') as f:
    f.write(code)


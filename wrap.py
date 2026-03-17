import re

with open('src/server/storage/sqlite.rs', 'r') as f:
    content = f.read()

def replacer(m):
    sig = m.group(1)
    body = m.group(2)
    return f"{sig}{{\n    tokio::task::block_in_place(|| {{\n{body}    }})\n}}"

# Match pub fn ... { ... }
# Need a proper parser. 

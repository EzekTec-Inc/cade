import re

with open("crates/cade-gui/src/session/mod.rs", "r") as f:
    mod = f.read()

# Extract the fields from `Connected { ... }`
match = re.search(r'Connected\s*\{([^}]+)\},', mod, re.DOTALL)
fields = match.group(1)

# Replace `Connected { ... }` with `Connected(Box<ConnectedSession>)`
mod = mod[:match.start()] + "Connected(Box<ConnectedSession>),\n" + mod[match.end():]

# Define `ConnectedSession` struct right above `pub enum SessionState`
struct_def = f"pub struct ConnectedSession {{{fields}}}\n\n"
enum_idx = mod.find("#[allow(clippy::large_enum_variant)]\npub enum SessionState")
mod = mod[:enum_idx] + struct_def + mod[enum_idx:]

with open("crates/cade-gui/src/session/mod.rs", "w") as f:
    f.write(mod)

print("Extracted ConnectedSession in mod.rs")

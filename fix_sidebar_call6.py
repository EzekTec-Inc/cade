import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

start = mod_rs.find('                            // ── Left sidebar: agent list ────────────────────')
end = mod_rs.find('                            // ── Plan panel', start)

if start != -1 and end != -1:
    mod_rs = mod_rs[:start] + mod_rs[end:]

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

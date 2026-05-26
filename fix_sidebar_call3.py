import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

# Replace draw_sidebar completely
start = mod_rs.find('    /// Renders the Left Sidebar')
end = mod_rs.find('    }\n', start) + 6
if start != -1 and end != -1:
    mod_rs = mod_rs[:start] + mod_rs[end:]

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

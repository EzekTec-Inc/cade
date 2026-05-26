import sys
import re

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

mod_rs = re.sub(r'                            if let Some\(new_action\) = self\.draw_sidebar\([\s\S]*?\)\n                            \{\n                                action = new_action;\n                            \}\n', '', mod_rs)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

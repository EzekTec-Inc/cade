import sys
import re

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

# Fix the old sidebar call that we somehow missed removing
mod_rs = re.sub(r'        components::sidebar::render\([^)]+\)\n', '', mod_rs, flags=re.DOTALL)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

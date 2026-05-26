import sys
import re

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

mod_rs = re.sub(r'    fn draw_sidebar\([^)]+\)[^{]+\{.*?components::sidebar::render\([^)]+\)\n    \}\n', '', mod_rs, flags=re.DOTALL)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

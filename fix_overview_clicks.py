import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/overview.rs', 'r') as f:
    overview = f.read()

overview = overview.replace('ui.add_sized([100.0, 32.0], save_btn);', 'if ui.add_sized([100.0, 32.0], save_btn).clicked() { }')
overview = overview.replace('ui.add_sized([110.0, 32.0], upgrade_btn);', 'if ui.add_sized([110.0, 32.0], upgrade_btn).clicked() { }')
overview = overview.replace('ui.add_sized([130.0, 32.0], retry_btn);', 'if ui.add_sized([130.0, 32.0], retry_btn).clicked() { }')
overview = overview.replace('ui.add(view_btn);', 'if ui.add(view_btn).clicked() { }')
overview = overview.replace('ui.add(btn_prev);', 'if ui.add(btn_prev).clicked() { }')
overview = overview.replace('ui.add(btn_next);', 'if ui.add(btn_next).clicked() { }')

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/overview.rs', 'w') as f:
    f.write(overview)


import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # Remove Copy and Mouse from SlashCmd match arm
    content = content.replace('        "copy" => Some(SlashCmd::Copy),\n', "")
    content = content.replace('        "mouse" | "select" => Some(SlashCmd::Mouse),\n', "")

    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-cli/src/cli/repl/slash.rs")

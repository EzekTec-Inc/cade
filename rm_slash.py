import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # Remove Copy and Mouse from SlashCmd enum
    content = content.replace("    Copy,\n", "")
    content = content.replace("    Mouse,\n", "")
    content = content.replace('"/copy" => Ok(SlashCmd::Copy),\n', "")
    content = content.replace('"/mouse" => Ok(SlashCmd::Mouse),\n', "")

    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-cli/src/cli/repl/slash.rs")

import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # In commands.rs
    if 'commands.rs' in path:
        content = content.replace("Copy,", "")
        content = content.replace('"/copy" => Ok(SlashCmd::Copy),', "")
        content = content.replace('SlashCmd::Copy => {\n                return self.cmd_copy().await;\n            }', "")
        content = content.replace('SlashCmd::Mouse => {\n                return self.cmd_mouse().await;\n            }', "")
        content = content.replace('Mouse,', '')
        content = content.replace('"/mouse" => Ok(SlashCmd::Mouse),', '')
    
    # In commands_session.rs
    if 'commands_session.rs' in path:
        # Regex to remove cmd_copy
        content = re.sub(r'pub\(crate\) async fn cmd_copy.*?Ok\(false\)\n    }', '', content, flags=re.DOTALL)

    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-cli/src/cli/repl/commands.rs")
fix_file("crates/cade-cli/src/cli/repl/commands_session.rs")

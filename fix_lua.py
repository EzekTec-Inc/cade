import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    old_code = """        if let Ok(package) = self.lua.globals().get::<mlua::Table>("package")
            && let Ok(current_path) = package.get::<String>("path")
        {
            let dir_str = plugin_dir.to_string_lossy();
            let new_path = format!(
                "{};{}/?.lua;{}/?/init.lua;./crates/cade-tui/?.lua;./cade-tui/?.lua",
                current_path, dir_str, dir_str
            );
            let _ = package.set("path", new_path);
        }

        let Ok(entries) = std::fs::read_dir(plugin_dir) else {
            return;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.is_dir() {
                    let init_path = path.join("init.lua");
                    if init_path.exists() {
                        Some(init_path)
                    } else {
                        None
                    }
                } else if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();"""

    new_code = """        let Ok(entries) = std::fs::read_dir(plugin_dir) else {
            return;
        };
        
        let mut dirs_with_init = Vec::new();

        let mut paths: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.is_dir() {
                    let init_path = path.join("init.lua");
                    if init_path.exists() {
                        dirs_with_init.push(path.to_string_lossy().into_owned());
                        Some(init_path)
                    } else {
                        None
                    }
                } else if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        // Add plugin_dir and cade-tui crate directories to package.path for require() routing
        if let Ok(package) = self.lua.globals().get::<mlua::Table>("package")
            && let Ok(current_path) = package.get::<String>("path")
        {
            let dir_str = plugin_dir.to_string_lossy();
            let mut new_path = format!(
                "{};{}/?.lua;{}/?/init.lua;./crates/cade-tui/?.lua;./cade-tui/?.lua",
                current_path, dir_str, dir_str
            );
            for d in dirs_with_init {
                new_path.push_str(&format!(";{}/?.lua;{}/?/init.lua", d, d));
            }
            let _ = package.set("path", new_path);
        }"""

    content = content.replace(old_code, new_code)
    
    with open(path, 'w') as f:
        f.write(content)

fix_file("crates/cade-tui/src/lua_engine.rs")

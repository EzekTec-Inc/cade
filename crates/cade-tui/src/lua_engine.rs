use mlua::prelude::*;

pub struct LuaEngine {
    pub lua: Lua,
}

impl LuaEngine {
    pub fn new() -> LuaResult<Self> {
        let lua = Lua::new();

        // Inject global state container
        let cade_state = lua.create_table()?;
        lua.globals().set("CADE_STATE", cade_state)?;

        // Simple print function for debugging plugins
        let print_fn = lua.create_function(|_, msg: String| {
            tracing::info!("[Lua] {}", msg);
            Ok(())
        })?;
        lua.globals().set("cade_log", print_fn)?;

        // UI Extensions table
        let ui_ext = lua.create_table()?;
        ui_ext.set("footer", "")?;
        lua.globals().set("CADE_UI", ui_ext)?;

        // CADE core table (for commands and keybindings)
        lua.load(r#"
            CADE = {
                _commands = {},
                _keybindings = {},
                register_command = function(name, cb)
                    CADE._commands[name] = cb
                end,
                bind_key = function(key, cb)
                    CADE._keybindings[key] = cb
                end,
                execute_slash_command = function(cmd)
                    cade_log("Lua requested slash command: " .. cmd)
                    -- For now, this is a placeholder. 
                    -- A full implementation would push this to a Rust channel.
                end,
                call_tool = function(name, args)
                    cade_log("Lua requested tool call: " .. name)
                    -- For now, this is a placeholder.
                end
            }
        "#).exec()?;

        Ok(Self { lua })
    }

    pub fn load_plugins(&self, plugin_dir: &std::path::Path) {
        if !plugin_dir.exists() {
            return;
        }

        // Add plugin_dir and cade-tui crate directories to package.path for require() routing
        if let Ok(package) = self.lua.globals().get::<mlua::Table>("package") {
            if let Ok(current_path) = package.get::<String>("path") {
                let dir_str = plugin_dir.to_string_lossy();
                let new_path = format!(
                    "{};{}/?.lua;{}/?/init.lua;./crates/cade-tui/?.lua;./cade-tui/?.lua", 
                    current_path, dir_str, dir_str
                );
                let _ = package.set("path", new_path);
            }
        }

        let Ok(entries) = std::fs::read_dir(plugin_dir) else { return };
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
            .collect();
            
        paths.sort();

        for path in paths {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Err(e) = self.lua.load(&content).set_name(path.to_string_lossy()).exec() {
                    tracing::warn!("Failed to load UI plugin {}: {}", path.display(), e);
                } else {
                    tracing::info!("Loaded UI plugin: {}", path.display());
                }
            }
        }
    }

    pub fn get_footer_text(&self) -> Option<String> {
        if let Ok(ui) = self.lua.globals().get::<mlua::Table>("CADE_UI") {
            if let Ok(footer) = ui.get::<String>("footer") {
                if !footer.is_empty() {
                    return Some(footer);
                }
            }
        }
        None
    }

    pub fn handle_keybinding(&self, key: &str) -> bool {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE") {
            if let Ok(bindings) = cade.get::<mlua::Table>("_keybindings") {
                if let Ok(func) = bindings.get::<mlua::Function>(key) {
                    if let Err(e) = func.call::<()>(()) {
                        tracing::warn!("Lua keybinding error for {}: {}", key, e);
                    }
                    return true; // handled
                }
            }
        }
        false
    }

    pub fn handle_command(&self, command: &str, args: Vec<String>) -> bool {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE") {
            if let Ok(commands) = cade.get::<mlua::Table>("_commands") {
                if let Ok(func) = commands.get::<mlua::Function>(command) {
                    if let Err(e) = func.call::<()>(args) {
                        tracing::warn!("Lua command error for {}: {}", command, e);
                    }
                    return true; // handled
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_multiple_plugins() {
        let engine = LuaEngine::new().unwrap();
        let dir = tempdir().unwrap();
        
        // 1. Standalone lua file
        std::fs::write(dir.path().join("1-standalone.lua"), "CADE_UI.footer = 'p1'").unwrap();
        
        // 2. Folder-based plugin with init.lua and internal require routing
        let folder_plugin = dir.path().join("2-folder");
        std::fs::create_dir(&folder_plugin).unwrap();
        std::fs::write(folder_plugin.join("helper.lua"), "return 'p2'").unwrap();
        std::fs::write(folder_plugin.join("init.lua"), "
            local helper = require('helper')
            CADE_UI.footer = CADE_UI.footer .. helper
        ").unwrap();
        
        engine.load_plugins(dir.path());
        
        assert_eq!(engine.get_footer_text().unwrap(), "p1p2");
    }
}

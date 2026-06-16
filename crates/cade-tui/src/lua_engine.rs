use mlua::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub struct LuaEngine {
    pub lua: Lua,
    pub command_queue: Arc<Mutex<VecDeque<String>>>,
    pub tool_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
    pub ui_event_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
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

        let command_queue = Arc::new(Mutex::new(VecDeque::new()));
        let tool_queue = Arc::new(Mutex::new(VecDeque::new()));
        let ui_event_queue = Arc::new(Mutex::new(VecDeque::new()));

        let cmd_q = command_queue.clone();
        let exec_cmd = lua.create_function(move |_, cmd: String| {
            tracing::info!("[Lua] execute_slash_command: {}", cmd);
            cmd_q.lock().unwrap().push_back(cmd);
            Ok(())
        })?;
        lua.globals().set("_CADE_execute_slash_command", exec_cmd)?;

        let tool_q = tool_queue.clone();
        let call_tool_fn =
            lua.create_function(move |lua_ctx, (name, args): (String, mlua::Value)| {
                let args_json: serde_json::Value = lua_ctx.from_value(args)?;
                tracing::info!("[Lua] call_tool: {} with args {}", name, args_json);
                tool_q.lock().unwrap().push_back((name, args_json));
                Ok(())
            })?;
        lua.globals().set("_CADE_call_tool", call_tool_fn)?;

        // UI Extensions table
        let ui_ext = lua.create_table()?;
        ui_ext.set("footer", "")?;
        lua.globals().set("CADE_UI", ui_ext)?;

        // CADE core table (for commands and keybindings)
        lua.load(
            r#"
            CADE = {
                _commands = {},
                _keybindings = {},
                _ui_callbacks = {},
                _hooks = {},
                register_command = function(name, cb)
                    CADE._commands[name] = cb
                end,
                bind_key = function(key, cb)
                    CADE._keybindings[key] = cb
                end,
                bind_ui_callback = function(id, cb)
                    CADE._ui_callbacks[id] = cb
                end,
                register_hook = function(event, cb)
                    CADE._hooks[event] = cb
                end,
                execute_slash_command = function(cmd)
                    _CADE_execute_slash_command(cmd)
                end,
                call_tool = function(name, args)
                    _CADE_call_tool(name, args)
                end
            }
        "#,
        )
        .exec()?;

        Ok(Self {
            lua,
            command_queue,
            tool_queue,
            ui_event_queue,
        })
    }

    pub fn set_state_u8(&self, key: &str, value: u8) -> mlua::Result<()> {
        let state: mlua::Table = self.lua.globals().get("CADE_STATE")?;
        state.set(key, value)?;
        Ok(())
    }

    pub fn set_state_nil(&self, key: &str) -> mlua::Result<()> {
        let state: mlua::Table = self.lua.globals().get("CADE_STATE")?;
        state.set(key, mlua::Value::Nil)?;
        Ok(())
    }

    pub fn load_plugins(&self, plugin_dir: &std::path::Path) {
        if !plugin_dir.exists() {
            return;
        }

        // Add plugin_dir and cade-tui crate directories to package.path for require() routing
        let Ok(entries) = std::fs::read_dir(plugin_dir) else {
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
        }

        paths.sort();

        for path in paths {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Err(e) = self
                    .lua
                    .load(&content)
                    .set_name(path.to_string_lossy())
                    .exec()
                {
                    tracing::warn!("Failed to load UI plugin {}: {}", path.display(), e);
                } else {
                    tracing::info!("Loaded UI plugin: {}", path.display());
                }
            }
        }
    }

    pub fn get_footer_text(&self) -> Option<String> {
        if let Ok(ui) = self.lua.globals().get::<mlua::Table>("CADE_UI")
            && let Ok(footer) = ui.get::<String>("footer")
            && !footer.is_empty()
        {
            return Some(footer);
        }
        None
    }

    pub fn handle_keybinding(&self, key: &str) -> bool {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE")
            && let Ok(bindings) = cade.get::<mlua::Table>("_keybindings")
            && let Ok(func) = bindings.get::<mlua::Function>(key)
        {
            if let Err(e) = func.call::<()>(()) {
                tracing::warn!("Lua keybinding error for {}: {}", key, e);
            }
            return true; // handled
        }
        false
    }

    pub fn handle_command(&self, command: &str, args: Vec<String>) -> bool {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE")
            && let Ok(commands) = cade.get::<mlua::Table>("_commands")
            && let Ok(func) = commands.get::<mlua::Function>(command)
        {
            if let Err(e) = func.call::<()>(args) {
                tracing::warn!("Lua command error for {}: {}", command, e);
            }
            return true; // handled
        }
        false
    }

    pub fn get_sidebar_ui(&self) -> Option<Vec<crate::lua_ui::LuaWidget>> {
        if let Ok(ui) = self.lua.globals().get::<mlua::Table>("CADE_UI")
            && let Ok(sidebar) = ui.get::<mlua::Value>("sidebar")
        {
            match self
                .lua
                .from_value::<Vec<crate::lua_ui::LuaWidget>>(sidebar)
            {
                Ok(widgets) => return Some(widgets),
                Err(e) => tracing::warn!("Failed to deserialize CADE_UI.sidebar: {}", e),
            }
        }
        None
    }

    pub fn get_header_ui(&self) -> Option<Vec<crate::lua_ui::LuaWidget>> {
        if let Ok(ui) = self.lua.globals().get::<mlua::Table>("CADE_UI") {
            if let Ok(header) = ui.get::<mlua::Value>("header") {
                match self.lua.from_value::<Vec<crate::lua_ui::LuaWidget>>(header) {
                    Ok(widgets) => return Some(widgets),
                    Err(e) => tracing::error!("Failed to deserialize CADE_UI.header: {}", e),
                }
            } else {
                tracing::error!("CADE_UI has no 'header' field");
            }
        } else {
            tracing::error!("CADE_UI global table missing");
        }
        None
    }

    pub fn trigger_mcp_ui(&self, uri: &str) -> bool {
        if let Ok(func) = self
            .lua
            .globals()
            .get::<mlua::Function>("CADE_TRIGGER_MCP_UI")
        {
            if let Err(e) = func.call::<()>(uri) {
                tracing::warn!("Lua CADE_TRIGGER_MCP_UI error: {}", e);
            }
            return true;
        }
        false
    }

    pub fn handle_ui_event(&self, id: &str, args: serde_json::Value) -> bool {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE")
            && let Ok(cbs) = cade.get::<mlua::Table>("_ui_callbacks")
            && let Ok(func) = cbs.get::<mlua::Function>(id)
        {
            let lua_args = if args.is_null() {
                mlua::Value::Nil
            } else {
                self.lua.to_value(&args).unwrap_or(mlua::Value::Nil)
            };
            if let Err(e) = func.call::<()>(lua_args) {
                tracing::warn!("Lua UI callback error for {}: {}", id, e);
            }
            return true;
        }
        false
    }

    pub fn run_hook(&self, hook_name: &str, input: serde_json::Value) -> Option<String> {
        if let Ok(cade) = self.lua.globals().get::<mlua::Table>("CADE")
            && let Ok(hooks) = cade.get::<mlua::Table>("_hooks")
            && let Ok(func) = hooks.get::<mlua::Function>(hook_name)
        {
            let input_lua = match self.lua.to_value(&input) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        "Failed to convert JSON input to Lua for hook {}: {}",
                        hook_name,
                        e
                    );
                    return None;
                }
            };
            match func.call::<Option<String>>(input_lua) {
                Ok(res) => return res,
                Err(e) => {
                    tracing::warn!("Lua hook error for {}: {}", hook_name, e);
                }
            }
        }
        None
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
        std::fs::write(
            folder_plugin.join("init.lua"),
            "
            local helper = require('helper')
            CADE_UI.footer = CADE_UI.footer .. helper
        ",
        )
        .unwrap();

        engine.load_plugins(dir.path());

        assert_eq!(engine.get_footer_text().unwrap(), "p1p2");
    }

    #[test]
    fn test_lua_hooks_register_and_run() {
        let engine = LuaEngine::new().unwrap();
        let dir = tempdir().unwrap();

        let hook_lua = r#"
            CADE.register_hook("pre_tool_use", function(input)
                if input.tool_name == "forbidden_tool" then
                    return "block: reason"
                else
                    return "allow"
                end
            end)
        "#;
        std::fs::write(dir.path().join("hook.lua"), hook_lua).unwrap();

        engine.load_plugins(dir.path());

        let input_block = serde_json::json!({
            "tool_name": "forbidden_tool"
        });
        let res_block = engine.run_hook("pre_tool_use", input_block);
        assert_eq!(res_block, Some("block: reason".to_string()));

        let input_allow = serde_json::json!({
            "tool_name": "allowed_tool"
        });
        let res_allow = engine.run_hook("pre_tool_use", input_allow);
        assert_eq!(res_allow, Some("allow".to_string()));
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_header_clock() {
        let engine = LuaEngine::new().unwrap();
        let dir = tempdir().unwrap();

        std::fs::write(
            dir.path().join("clock.lua"),
            "
            CADE_UI.header = {
                { type = 'clock', format = '%H:%M:%S', color = 'cyan' }
            }
        ",
        )
        .unwrap();

        engine.load_plugins(dir.path());

        let header = engine.get_header_ui();
        assert!(header.is_some(), "Header was None!");
    }
}

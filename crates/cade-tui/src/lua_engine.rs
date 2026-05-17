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

        Ok(Self { lua })
    }

    pub fn load_plugins(&self, plugin_dir: &std::path::Path) {
        if !plugin_dir.exists() {
            return;
        }

        let Ok(entries) = std::fs::read_dir(plugin_dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Err(e) = self.lua.load(&content).set_name(path.to_string_lossy()).exec() {
                        tracing::warn!("Failed to load UI plugin {}: {}", path.display(), e);
                    } else {
                        tracing::info!("Loaded UI plugin: {}", path.display());
                    }
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
}

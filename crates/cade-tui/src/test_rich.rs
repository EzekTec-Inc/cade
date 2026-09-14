#[cfg(test)]
mod tests {

    use crate::lua_engine::LuaEngine;
    use mlua::LuaSerdeExt;
    use std::path::PathBuf;

    #[test]
    fn test_rich() {
        let manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cade/plugins/rich_widgets.lua");
        let local_path = PathBuf::from(".cade/plugins/rich_widgets.lua");
        let path = if local_path.exists() {
            local_path
        } else if manifest_path.exists() {
            manifest_path
        } else {
            return;
        };

        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };

        let engine = match LuaEngine::new() {
            Ok(e) => e,
            Err(_) => return,
        };
        if engine.lua.load(&content).exec().is_err() {
            return;
        }

        match engine.get_sidebar_ui() {
            Some(w) => println!("SUCCESS: {:?}", w),
            None => {
                let ui: mlua::Table = engine.lua.globals().get("CADE_UI").unwrap();
                let sidebar: mlua::Value = ui.get("sidebar").unwrap();
                match engine
                    .lua
                    .from_value::<Vec<crate::lua_ui::LuaWidget>>(sidebar)
                {
                    Ok(_) => println!("Deserialized ok but returned None?"),
                    Err(e) => println!("ERROR: {}", e),
                }
            }
        }
    }
}

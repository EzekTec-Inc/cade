#[cfg(test)]
mod tests {

    use super::*;
    use crate::lua_engine::LuaEngine;
    use mlua::LuaSerdeExt;
    use std::path::PathBuf;

    #[test]
    fn test_rich() {
        let engine = LuaEngine::new().unwrap();
        let path = PathBuf::from("../../.cade/plugins/rich_widgets.lua");
        let content = std::fs::read_to_string(&path).unwrap();
        engine.lua.load(&content).exec().unwrap();

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

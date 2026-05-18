use mlua::prelude::*;

fn main() -> LuaResult<()> {
    let lua = Lua::new();
    lua.load(include_str!("test_mlua.lua")).exec()?;
    let ui: mlua::Table = lua.globals().get("CADE_UI")?;
    let sidebar: mlua::Value = ui.get("sidebar")?;
    println!("sidebar: {:?}", sidebar);
    Ok(())
}

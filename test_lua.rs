fn main() {
    let engine = cade_tui::lua_engine::LuaEngine::new().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let plugin_dir = cwd.join(".cade").join("plugins");
    println!("Loading plugins from {:?}", plugin_dir);
    engine.load_plugins(&plugin_dir);
    let header = engine.get_header_ui();
    println!("Header: {:?}", header);
}

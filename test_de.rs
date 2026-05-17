fn main() {
    let json = r#"
    [
        {
            "type": "clock",
            "format": "%H:%M:%S",
            "color": "cyan"
        }
    ]
    "#;
    let w: Result<Vec<cade_tui::lua_ui::LuaWidget>, _> = serde_json::from_str(json);
    println!("{:?}", w);
}
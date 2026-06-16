fn main() {
    let json_theme = r#"
    {
        "meta": {
            "name": "my-json",
            "variant": "dark"
        },
        "colors": {}
    }
    "#;
    let res = opaline::load_from_str(json_theme, None);
    println!("JSON: {:?}", res);

    let toml_theme = r#"
    [meta]
    name = "my-toml"
    variant = "dark"
    [colors]
    "#;
    let res2 = opaline::load_from_str(toml_theme, None);
    println!("TOML: {:?}", res2);
}

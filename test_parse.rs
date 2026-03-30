use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ThemeColor { Hex(String), Index(u8) }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThemeTokens {
    pub accent: ThemeColor,
    pub border: ThemeColor,
    #[serde(rename = "mdHeading")]
    pub md_heading: ThemeColor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeTokens,
}

fn main() {
    let json = r#"{"name": "test", "colors": {"accent": "#ffffff", "border": "#000000"}}"#;
    let res: Result<Theme, _> = serde_json::from_str(json);
    println!("{:?}", res);
}

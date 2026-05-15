use std::path::PathBuf;
#[derive(Debug, Clone)]
pub enum MemberTools {
    All,
    Readonly,
    List(Vec<String>),
    Restricted {
        allowed_tools: Vec<String>,
        allowed_paths: Vec<String>,
    },
}
impl std::fmt::Display for MemberTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Readonly => write!(f, "readonly"),
            Self::List(v) => write!(f, "{}", v.join(", ")),
            Self::Restricted {
                allowed_tools,
                allowed_paths,
            } => write!(
                f,
                "restricted (tools: [{}], paths: [{}])",
                allowed_tools.join(", "),
                allowed_paths.join(", ")
            ),
        }
    }
}
impl MemberTools {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "all" => Self::All,
            "readonly" | "read-only" | "read_only" => Self::Readonly,
            other => {
                if other.starts_with('{')
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(other)
                    && let (Some(tools), Some(paths)) = (
                        v.get("allowed_tools").and_then(|v| v.as_array()),
                        v.get("allowed_paths").and_then(|v| v.as_array()),
                    )
                {
                    return Self::Restricted {
                        allowed_tools: tools
                            .iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect(),
                        allowed_paths: paths
                            .iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect(),
                    };
                }
                Self::List(
                    other
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect(),
                )
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemberScope {
    Builtin = 0,
    Global = 1,
    Project = 2,
}
impl std::fmt::Display for MemberScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global => write!(f, "global"),
            Self::Project => write!(f, "project"),
        }
    }
}
#[derive(Debug, Clone)]
pub struct MemberDef {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub description: String,
    pub model: Option<String>,
    pub tools: MemberTools,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub scope: MemberScope,
    pub path: Option<PathBuf>,
}
impl MemberDef {
    pub fn summary(&self) -> String {
        format!(
            "  [{:<8}] {:<22} — {} [{}]",
            self.scope, self.id, self.description, self.tools
        )
    }
    pub fn to_xml_description(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut out = format!("{pad}<member id=\"{}\" name=\"{}\">\n", self.id, self.name);
        if let Some(role) = &self.role {
            out.push_str(&format!("{pad}  Role: {role}\n"));
        }
        out.push_str(&format!(
            "{pad}  Description: {}\n{pad}  Tools: {}\n{pad}</member>\n",
            self.description, self.tools
        ));
        out
    }
}

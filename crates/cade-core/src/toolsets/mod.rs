/// Which family of tools to attach to the agent.
/// Different model families are trained with different editing paradigms —
/// mismatching them produces degraded edit quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Toolset {
    /// String-replace editing — optimised for Claude (Anthropic) models.
    #[default]
    Default,
    /// Patch-based editing (unified diff) — optimised for OpenAI (GPT/Codex) models.
    Codex,
    /// String-replace variant — optimised for Google (Gemini) models.
    Gemini,
}

impl Toolset {
    /// Detect the best toolset from a model identifier string.
    pub fn for_model(model: &str) -> Self {
        let m = model.to_lowercase();
        if m.contains("gpt")
            || m.contains("codex")
            || m.contains("o1")
            || m.contains("o3")
            || m.contains("o4")
        {
            Self::Codex
        } else if m.contains("gemini") {
            Self::Gemini
        } else {
            Self::Default // Claude, Llama, Mistral, etc.
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "Default (string-replace, Claude/Anthropic)",
            Self::Codex   => "Codex (patch-based, OpenAI/GPT)",
            Self::Gemini  => "Gemini (string-replace, Google)",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Codex   => "codex",
            Self::Gemini  => "gemini",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "default" | "claude" | "anthropic" => Some(Self::Default),
            "codex" | "openai" | "gpt"         => Some(Self::Codex),
            "gemini" | "google"                 => Some(Self::Gemini),
            _                                   => None,
        }
    }

    /// The file-editing tool name for this toolset.
    pub fn edit_tool(&self) -> &'static str {
        match self {
            Self::Codex => "apply_patch",
            Self::Gemini => "Replace",
            _           => "edit_file",
        }
    }

    /// Core tool names for this toolset (excludes meta-tools: memory, skills, subagents).
    pub fn core_tool_names(&self) -> &'static [&'static str] {
        match self {
            Self::Default => &[
                "bash",
                "read_file",
                "write_file",
                "edit_file",
                "grep",
                "glob",
                "desktop_screenshot",
                "desktop_list_windows",
                "desktop_control",
                "desktop_notify",
            ],
            Self::Gemini => &[
                "RunShellCommand",
                "ReadFileGemini",
                "WriteFileGemini",
                "Replace",
                "SearchFileContent",
                "GlobGemini",
                "desktop_screenshot",
                "desktop_list_windows",
                "desktop_control",
                "desktop_notify",
            ],
            Self::Codex => &[
                "bash",
                "read_file",
                "apply_patch",   // replaces edit_file + write_file for Codex
                "grep",
                "glob",
                "desktop_screenshot",
                "desktop_list_windows",
                "desktop_control",
                "desktop_notify",
            ],
        }
    }

    /// Meta-tool names for this toolset.
    pub fn meta_tool_names(&self) -> &'static [&'static str] {
        match self {
            Self::Codex => &[
                "memory_apply_patch",
                "load_skill",
                "install_skill",
                "run_skill_script",
                "load_skill_ref",
                "run_subagent",
            ],
            _ => &[
                "update_memory",
                "load_skill",
                "install_skill",
                "run_skill_script",
                "load_skill_ref",
                "run_subagent",
            ],
        }
    }

    /// All tool names for this toolset (core + meta).
    pub fn all_tool_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.core_tool_names().to_vec();
        names.extend_from_slice(self.meta_tool_names());
        names
    }
}

impl std::fmt::Display for Toolset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Toolset::for_model ────────────────────────────────────────────────

    #[test]
    fn for_model_claude() {
        assert_eq!(Toolset::for_model("claude-sonnet-4-5-20250929"), Toolset::Default);
        assert_eq!(Toolset::for_model("claude-3-opus-20240229"), Toolset::Default);
    }

    #[test]
    fn for_model_gpt() {
        assert_eq!(Toolset::for_model("gpt-4o"), Toolset::Codex);
        assert_eq!(Toolset::for_model("gpt-4o-mini"), Toolset::Codex);
        assert_eq!(Toolset::for_model("GPT-4.1"), Toolset::Codex);
    }

    #[test]
    fn for_model_openai_reasoning() {
        assert_eq!(Toolset::for_model("o1-preview"), Toolset::Codex);
        assert_eq!(Toolset::for_model("o3-mini"), Toolset::Codex);
        assert_eq!(Toolset::for_model("o4-mini"), Toolset::Codex);
    }

    #[test]
    fn for_model_gemini() {
        assert_eq!(Toolset::for_model("gemini-2.5-pro"), Toolset::Gemini);
        assert_eq!(Toolset::for_model("gemini-1.5-flash"), Toolset::Gemini);
    }

    #[test]
    fn for_model_other_defaults() {
        assert_eq!(Toolset::for_model("llama-3-70b"), Toolset::Default);
        assert_eq!(Toolset::for_model("mistral-large"), Toolset::Default);
        assert_eq!(Toolset::for_model("unknown-model"), Toolset::Default);
    }

    // ── Toolset::from_str ─────────────────────────────────────────────────

    #[test]
    fn from_str_valid() {
        assert_eq!(Toolset::from_str("default"), Some(Toolset::Default));
        assert_eq!(Toolset::from_str("claude"), Some(Toolset::Default));
        assert_eq!(Toolset::from_str("anthropic"), Some(Toolset::Default));
        assert_eq!(Toolset::from_str("codex"), Some(Toolset::Codex));
        assert_eq!(Toolset::from_str("openai"), Some(Toolset::Codex));
        assert_eq!(Toolset::from_str("gpt"), Some(Toolset::Codex));
        assert_eq!(Toolset::from_str("gemini"), Some(Toolset::Gemini));
        assert_eq!(Toolset::from_str("google"), Some(Toolset::Gemini));
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(Toolset::from_str("DEFAULT"), Some(Toolset::Default));
        assert_eq!(Toolset::from_str("Codex"), Some(Toolset::Codex));
        assert_eq!(Toolset::from_str("GEMINI"), Some(Toolset::Gemini));
    }

    #[test]
    fn from_str_unknown() {
        assert_eq!(Toolset::from_str("unknown"), None);
        assert_eq!(Toolset::from_str(""), None);
    }

    // ── Toolset names and schemas ─────────────────────────────────────────

    #[test]
    fn edit_tool_per_toolset() {
        assert_eq!(Toolset::Default.edit_tool(), "edit_file");
        assert_eq!(Toolset::Codex.edit_tool(), "apply_patch");
        assert_eq!(Toolset::Gemini.edit_tool(), "Replace");
    }

    #[test]
    fn core_tools_non_empty() {
        assert!(!Toolset::Default.core_tool_names().is_empty());
        assert!(!Toolset::Codex.core_tool_names().is_empty());
        assert!(!Toolset::Gemini.core_tool_names().is_empty());
    }

    #[test]
    fn meta_tools_non_empty() {
        assert!(!Toolset::Default.meta_tool_names().is_empty());
        assert!(!Toolset::Codex.meta_tool_names().is_empty());
        assert!(!Toolset::Gemini.meta_tool_names().is_empty());
    }

    #[test]
    fn all_tool_names_is_core_plus_meta() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            let all = ts.all_tool_names();
            let core = ts.core_tool_names();
            let meta = ts.meta_tool_names();
            assert_eq!(all.len(), core.len() + meta.len());
            for name in core { assert!(all.contains(name)); }
            for name in meta { assert!(all.contains(name)); }
        }
    }

    #[test]
    fn codex_has_apply_patch_not_write_file() {
        let names = Toolset::Codex.core_tool_names();
        assert!(names.contains(&"apply_patch"));
        assert!(!names.contains(&"write_file"));
    }

    #[test]
    fn default_has_edit_file_not_apply_patch() {
        let names = Toolset::Default.core_tool_names();
        assert!(names.contains(&"edit_file"));
        assert!(!names.contains(&"apply_patch"));
    }

    // ── Display ───────────────────────────────────────────────────────────

    #[test]
    fn display_short_name() {
        assert_eq!(Toolset::Default.to_string(), "default");
        assert_eq!(Toolset::Codex.to_string(), "codex");
        assert_eq!(Toolset::Gemini.to_string(), "gemini");
    }

    #[test]
    fn display_name() {
        assert!(Toolset::Default.display_name().contains("Claude"));
        assert!(Toolset::Codex.display_name().contains("OpenAI"));
        assert!(Toolset::Gemini.display_name().contains("Google"));
    }

    // ── Default trait ─────────────────────────────────────────────────────

    #[test]
    fn default_is_default() {
        assert_eq!(Toolset::default(), Toolset::Default);
    }
}

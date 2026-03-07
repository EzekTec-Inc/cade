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
            _           => "edit_file",
        }
    }

    /// Core tool names for this toolset (excludes meta-tools: memory, skills, subagents).
    pub fn core_tool_names(&self) -> &'static [&'static str] {
        match self {
            Self::Default | Self::Gemini => &[
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

    /// Meta-tool names — same for every toolset.
    pub fn meta_tool_names() -> &'static [&'static str] {
        &[
            "update_memory",
            "load_skill",
            "install_skill",
            "run_skill_script",
            "load_skill_ref",
            "run_subagent",
        ]
    }

    /// All tool names for this toolset (core + meta).
    pub fn all_tool_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.core_tool_names().to_vec();
        names.extend_from_slice(Self::meta_tool_names());
        names
    }
}

impl std::fmt::Display for Toolset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short_name())
    }
}

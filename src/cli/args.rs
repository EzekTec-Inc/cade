use clap::Parser;

/// CADE — Coding AI assistant with Desktop Extensions
#[derive(Parser, Debug)]
#[command(name = "cade", version, about)]
pub struct Args {
    /// Headless prompt (non-interactive mode)
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Start a fresh conversation on the current agent (does not create a new agent)
    #[arg(long = "new", conflicts_with = "agent")]
    pub new_conversation: bool,

    /// Create a new agent
    #[arg(long = "new-agent")]
    pub new_agent: bool,

    /// Browse past conversations interactively and resume one
    #[arg(long = "resume")]
    pub resume: bool,

    /// Resume last session exactly (suppresses first-turn env context re-injection)
    #[arg(long = "continue", short = 'c')]
    pub continue_last: bool,

    /// Connect to a specific agent by ID
    #[arg(long = "agent", short = 'a')]
    pub agent: Option<String>,

    /// Resume agent by name (partial, case-insensitive match)
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,

    /// Model to use in provider/model format (e.g., anthropic/claude-sonnet-4-5-20250929)
    #[arg(short = 'm', long = "model")]
    pub model: Option<String>,

    /// Reasoning tier for models that support it (none, low, medium, high, xhigh)
    #[arg(long = "reasoning")]
    pub reasoning: Option<String>,

    /// Bypass all permission prompts (use carefully)
    #[arg(long = "yolo")]
    pub yolo: bool,

    /// Permission mode: default | acceptEdits | plan | bypassPermissions
    #[arg(long = "permission-mode")]
    pub permission_mode: Option<String>,

    /// Enable system tray icon (runs CADE in background)
    #[arg(long = "tray")]
    pub tray: bool,

    /// Custom skills directory
    #[arg(long = "skills")]
    pub skills: Option<String>,

    /// Show connection info and current agent (without starting a session)
    #[arg(long = "info")]
    pub info: bool,

    /// Always-allow tool patterns (comma-separated): "Bash(cargo test),Read"
    #[arg(long = "allowed-tools")]
    pub allowed_tools: Option<String>,

    /// Always-deny tool patterns (comma-separated): "Bash(rm -rf:*)"
    #[arg(long = "disallowed-tools")]
    pub disallowed_tools: Option<String>,

    /// Output format: text | json | stream-json (headless mode only)
    #[arg(long = "output-format")]
    pub output_format: Option<String>,

    /// Disable streaming (headless mode only)
    #[arg(long = "no-stream")]
    pub no_stream: bool,

    /// Force a specific toolset: default | codex | gemini
    #[arg(long = "toolset")]
    pub toolset: Option<String>,

    /// Rename the resolved agent and exit (combine with --agent or --name to target a specific one)
    #[arg(long = "rename", short = 'r')]
    pub rename: Option<String>,

    /// Restrict which tools are attached to the agent (comma-separated names, or "" for none).
    /// Unlike --allowed-tools (runtime permission gate), this controls what is registered
    /// in the LLM's context window. Example: --tools "bash,read_file,grep"
    #[arg(long = "tools")]
    pub tools: Option<String>,

    /// Attach CADE tools to the agent and start the session (re-links tools after /unlink)
    #[arg(long = "link")]
    pub link: bool,

    /// Remove all CADE tools from the agent and start the session
    #[arg(long = "unlink")]
    pub unlink: bool,

    /// Maximum wall-clock time for headless mode in seconds (0 = no limit).
    /// On timeout the process exits with code 124 (standard timeout exit code).
    #[arg(long = "timeout-secs", default_value_t = 0)]
    pub timeout_secs: u64,

    /// Export an agent to a JSON file. Value is the agent name or ID.
    /// Optionally combine with --output to set the output file (default: <name>-<timestamp>.json).
    /// Example: cade --export-agent my-agent --output backup.json
    #[arg(long = "export-agent", conflicts_with_all = ["prompt", "import_agent"])]
    pub export_agent: Option<String>,

    /// Output file for --export-agent (use "-" for stdout).
    #[arg(long = "output", short = 'o')]
    pub output: Option<String>,

    /// Import an agent from a JSON export file. Value is the path to the file (use "-" for stdin).
    /// Example: cade --import-agent backup.json
    #[arg(long = "import-agent", conflicts_with_all = ["prompt", "export_agent"])]
    pub import_agent: Option<String>,
}

impl Args {
    /// Parse --tools into an optional name list.
    /// None  → not specified (all tools)
    /// Some([]) → empty string → no tools
    /// Some(names) → filter to these names
    pub fn tool_filter(&self) -> Option<Vec<String>> {
        self.tools.as_ref().map(|s| {
            if s.is_empty() {
                vec![]
            } else {
                s.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
            }
        })
    }
}

impl Args {
    pub fn effective_permission_mode(&self) -> &str {
        if self.yolo {
            "bypassPermissions"
        } else {
            self.permission_mode.as_deref().unwrap_or("default")
        }
    }

    /// Returns the effective output format for headless mode.
    pub fn effective_output_format(&self) -> &str {
        self.output_format.as_deref().unwrap_or("text")
    }
}

use clap::Parser;

/// CADE — Coding AI assistant with Desktop Extensions
#[derive(Parser, Debug)]
#[command(name = "cade", version, about)]
pub struct Args {
    /// Headless prompt (non-interactive mode)
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Create a new agent
    #[arg(long = "new", conflicts_with = "agent")]
    pub new_agent: bool,

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

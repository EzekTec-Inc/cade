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

    /// Connect to a specific agent by ID
    #[arg(long = "agent", short = 'a')]
    pub agent: Option<String>,

    /// Model to use in provider/model format (e.g., anthropic/claude-sonnet-4-5-20250929, openai/gpt-4o)
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

    /// Show connection info and current agent
    #[arg(long = "info")]
    pub info: bool,

    /// Output response as JSON (headless mode only)
    #[arg(long = "json")]
    pub json: bool,

    /// Disable streaming (headless mode only — for clean JSON output)
    #[arg(long = "no-stream")]
    pub no_stream: bool,

    /// Force a specific toolset: default | codex | gemini
    #[arg(long = "toolset")]
    pub toolset: Option<String>,
}

impl Args {
    pub fn effective_permission_mode(&self) -> &str {
        if self.yolo {
            "bypassPermissions"
        } else {
            self.permission_mode.as_deref().unwrap_or("default")
        }
    }
}

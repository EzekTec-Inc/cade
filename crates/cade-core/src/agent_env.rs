use std::sync::OnceLock;

/// Stores the active agent ID so commands spawned by this process can expose it
/// to child processes via the `AGENT_ID` environment variable without touching
/// global process environment APIs (which are `unsafe` on Rust 2024).
static AGENT_ID: OnceLock<String> = OnceLock::new();

/// Set the agent ID once per process. Subsequent calls are ignored.
pub fn set_agent_id(id: impl Into<String>) {
    let _ = AGENT_ID.set(id.into());
}

/// Get the agent ID if it has been set.
pub fn agent_id() -> Option<&'static str> {
    AGENT_ID.get().map(|s| s.as_str())
}

/// Trait implemented for command builders that support `.env()`.
pub trait CommandAgentEnv {
    fn set_agent_env_var(&mut self, key: &str, value: &str);
}

impl CommandAgentEnv for std::process::Command {
    fn set_agent_env_var(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

impl CommandAgentEnv for tokio::process::Command {
    fn set_agent_env_var(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

/// Apply the agent environment (if any) to the command builder.
pub fn apply_agent_env<C: CommandAgentEnv>(cmd: &mut C) {
    if let Some(id) = agent_id() {
        cmd.set_agent_env_var("AGENT_ID", id);
    }
}

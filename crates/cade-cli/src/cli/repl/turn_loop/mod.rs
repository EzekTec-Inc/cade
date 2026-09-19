use super::Repl;
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};

#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct TurnStats {
    pub reads: u32,
    pub edits: u32,
    pub cmds: u32,
}

pub(crate) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub mod agent;
pub mod director;
pub mod env_context;
pub mod stream;

pub use director::{TurnDirector, TurnOutcome};

pub mod config;
pub mod context;
pub mod discovery;
pub mod member;
pub mod mode;
pub mod task;
pub use config::TeamConfig;
pub use context::{MemberInteraction, TeamRunContext};
pub use discovery::{TeamDef, discover_all_teams, find_team};
pub use member::{MemberDef, MemberScope, MemberTools};
pub use mode::TeamMode;
pub use task::{Task, TaskList, TaskStatus};
#[must_use]
pub fn should_emit_completion_bell(silent: bool, is_tty: bool) -> bool {
    !silent && is_tty
}
#[derive(Debug, Clone)]
pub struct BackgroundResult {
    pub task_id: String,
    pub subagent: String,
    pub prompt_preview: String,
    pub result: String,
    pub is_error: bool,
}

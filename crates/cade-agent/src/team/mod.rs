pub mod config;
pub mod context;
pub mod discovery;
pub mod executor;
pub mod member;
pub mod mode;
pub mod task;
pub use config::TeamConfig;
pub use context::{MemberInteraction, TeamRunContext};
pub use discovery::{TeamDef, discover_all_teams, find_team};
pub use executor::{LlmCompleter, SubagentRunner, TeamExecutor, TeamResultItem};
pub use member::{MemberDef, MemberScope, MemberTools};
pub use mode::TeamMode;
pub use task::{Task, TaskList, TaskStatus};
#[derive(Debug, Clone)]
pub struct BackgroundResult {
    pub task_id: String,
    pub subagent: String,
    pub prompt_preview: String,
    pub result: String,
    pub is_error: bool,
}

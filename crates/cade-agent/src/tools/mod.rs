// region:    --- Modules

pub mod ask;
pub mod bash;
pub mod catalog;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod fs;
pub mod git_checkpoint;
pub mod manager;
pub mod memory;
pub mod meta;
pub mod plan;
pub mod runtime;
pub mod search;

pub use ask::AskUserQuestionTool;
pub use manager::{
    ToolResult, all_schemas, dispatch, is_mcp_write_tool, schemas_for_names, schemas_for_toolset,
};
pub use meta::{all_meta_schemas, register_meta_tools};
pub use plan::{
    EnterPlanModeTool, ExitPlanModeTool, FinishTaskTool, SetPlanTool, TodoWriteTool, UpdatePlanTool,
};
pub use runtime::{RuntimeToolResult, ToolRuntime};

// endregion: --- Modules

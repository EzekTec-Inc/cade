#![allow(clippy::empty_line_after_doc_comments)]
/// Canonical tool identifiers used throughout CADE.
///
/// Every tool has one canonical ID.  Provider- or model-specific names
/// (e.g. "RunShellCommand" for Gemini, "apply_patch" for Codex) are
/// surface aliases managed by [`crate::toolsets::adapter::ToolSurfaceAdapter`].
/// All internal code should use these constants; only the LLM-facing schema
/// serialization layer uses the aliases.
// region:    --- Core coding tools

pub const BASH: &str = "bash";
pub const READ_FILE: &str = "read_file";
pub const WRITE_FILE: &str = "write_file";
pub const EDIT_FILE: &str = "edit_file";
pub const APPLY_PATCH: &str = "apply_patch";
pub const GREP: &str = "grep";
pub const GLOB: &str = "glob";

// endregion: --- Core coding tools

// region:    --- Desktop tools

pub const DESKTOP_SCREENSHOT: &str = "desktop_screenshot";
pub const DESKTOP_LIST_WINDOWS: &str = "desktop_list_windows";
pub const DESKTOP_CONTROL: &str = "desktop_control";
pub const DESKTOP_NOTIFY: &str = "desktop_notify";

// endregion: --- Desktop tools

// region:    --- Plan / task tools

pub const ENTER_PLAN_MODE: &str = "EnterPlanMode";
pub const EXIT_PLAN_MODE: &str = "ExitPlanMode";
pub const TODO_WRITE: &str = "TodoWrite";
pub const UPDATE_PLAN: &str = "UpdatePlan";
pub const WRITE_TODOS: &str = "WriteTodos";

// endregion: --- Plan / task tools

// region:    --- Meta tools (memory)

pub const UPDATE_MEMORY: &str = "update_memory";
pub const MEMORY_APPLY_PATCH: &str = "memory_apply_patch";
pub const SEARCH_MEMORY: &str = "search_memory";
pub const CONVERSATION_SEARCH: &str = "conversation_search";
pub const QUERY_EVENT_LOG: &str = "query_event_log";
pub const ARCHIVAL_MEMORY_INSERT: &str = "archival_memory_insert";
pub const ARCHIVAL_MEMORY_SEARCH: &str = "archival_memory_search";

// endregion: --- Meta tools (memory)

// region:    --- Meta tools (skills)

pub const LOAD_SKILL: &str = "load_skill";
pub const INSTALL_SKILL: &str = "install_skill";
pub const INSTALL_PLUGIN: &str = "install_plugin";
pub const RUN_SKILL_SCRIPT: &str = "run_skill_script";
pub const LOAD_SKILL_REF: &str = "load_skill_ref";

// endregion: --- Meta tools (skills)

// region:    --- Meta tools (subagents)

pub const RUN_SUBAGENT: &str = "run_subagent";
pub const LIST_AGENTS: &str = "list_agents";
pub const MESSAGE_AGENT: &str = "message_agent";

// endregion: --- Meta tools (subagents)

// region:    --- Interaction tools

pub const ASK_USER_QUESTION: &str = "ask_user_question";

// endregion: --- Interaction tools

// region:    --- Code intelligence (Phase 3)

pub const SYMBOL_SEARCH: &str = "symbol_search";
pub const FIND_REFERENCES: &str = "find_references";
pub const GOTO_DEFINITION: &str = "goto_definition";
pub const GET_REPO_MAP: &str = "get_repo_map";
pub const INDEX_REPOSITORY: &str = "index_repository";

// endregion: --- Code intelligence (Phase 3)

// region:    --- Checkpoints + artifacts (Phase 4)

pub const CREATE_CHECKPOINT: &str = "create_checkpoint";
pub const RESTORE_CHECKPOINT: &str = "restore_checkpoint";
pub const STORE_ARTIFACT: &str = "store_artifact";
pub const LIST_CHECKPOINTS: &str = "list_checkpoints";

// endregion: --- Checkpoints + artifacts (Phase 4)

// region:    --- Typed memory / provenance / reflection (Phase 5)

pub const UPDATE_MEMORY_TYPED: &str = "update_memory_typed";
pub const UPDATE_MEMORY_FIELD: &str = "update_memory_field";
pub const LINK_MEMORY_EVIDENCE: &str = "link_memory_evidence";
pub const REFLECT: &str = "reflect";

// endregion: --- Checkpoints + artifacts (Phase 4)

// region:    --- Web tools (Phase 6)

pub const WEB_SEARCH: &str = "web_search";
pub const FETCH_DOC: &str = "fetch_doc";
pub const BROWSER_SCREENSHOT: &str = "browser_screenshot";

// endregion: --- Web tools (Phase 6)

// region:    --- Support

/// All canonical memory tool names.  Used by callers that need to
/// check "is this a memory tool?" without enumerating individually.
pub const MEMORY_TOOL_IDS: &[&str] = &[
    UPDATE_MEMORY,
    MEMORY_APPLY_PATCH,
    SEARCH_MEMORY,
    CONVERSATION_SEARCH,
    ARCHIVAL_MEMORY_INSERT,
    ARCHIVAL_MEMORY_SEARCH,
    UPDATE_MEMORY_FIELD,
];

// endregion: --- Typed memory / provenance / reflection (Phase 5)

/// All meta tool names (memory + skills + subagents + checkpoints + artifacts + typed memory).
pub const META_TOOL_IDS: &[&str] = &[
    UPDATE_MEMORY,
    MEMORY_APPLY_PATCH,
    SEARCH_MEMORY,
    CONVERSATION_SEARCH,
    ARCHIVAL_MEMORY_INSERT,
    ARCHIVAL_MEMORY_SEARCH,
    LOAD_SKILL,
    INSTALL_SKILL,
    RUN_SKILL_SCRIPT,
    LOAD_SKILL_REF,
    RUN_SUBAGENT,
    LIST_AGENTS,
    MESSAGE_AGENT,
    CREATE_CHECKPOINT,
    LIST_CHECKPOINTS,
    RESTORE_CHECKPOINT,
    STORE_ARTIFACT,
    UPDATE_MEMORY_TYPED,
    UPDATE_MEMORY_FIELD,
    LINK_MEMORY_EVIDENCE,
    REFLECT,
    BROWSER_SCREENSHOT,
    WEB_SEARCH,
    FETCH_DOC,
];

// endregion: --- Support

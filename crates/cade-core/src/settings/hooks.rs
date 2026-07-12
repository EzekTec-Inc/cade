use serde::{Deserialize, Serialize};

// -- Hook configuration

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HookDef {
    /// Run a shell command. Exit 0=allow, 1=log+continue, 2=block+stderr→agent.
    Command {
        command: String,
        #[serde(default = "default_hook_timeout")]
        timeout: u64, // milliseconds
    },
    /// Blocks mutating or access operations containing forbidden paths.
    PathBlocker {
        forbidden_paths: Vec<String>,
    },
    /// Scans inputs, prompts, or arguments for blacklisted regex patterns.
    RegexGuard {
        patterns: Vec<String>,
    },
}

fn default_hook_timeout() -> u64 {
    60_000
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookEntry {
    /// Regex matcher for tool name (tool-related hooks only).
    /// None / "" / "*" → match all tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookDef>,
}

/// All hooks grouped by event type.
/// Field names match CADE's settings.json key names exactly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default, rename = "PreToolUse")]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(default, rename = "PostToolUse")]
    pub post_tool_use: Vec<HookEntry>,
    #[serde(default, rename = "PostToolUseFailure")]
    pub post_tool_use_failure: Vec<HookEntry>,
    #[serde(default, rename = "PermissionRequest")]
    pub permission_request: Vec<HookEntry>,
    #[serde(default, rename = "UserPromptSubmit")]
    pub user_prompt_submit: Vec<HookEntry>,
    #[serde(default, rename = "Stop")]
    pub stop: Vec<HookEntry>,
    #[serde(default, rename = "SubagentStop")]
    pub subagent_stop: Vec<HookEntry>,
    #[serde(default, rename = "SessionStart")]
    pub session_start: Vec<HookEntry>,
    #[serde(default, rename = "SessionEnd")]
    pub session_end: Vec<HookEntry>,
    #[serde(default, rename = "Notification")]
    pub notification: Vec<HookEntry>,
}

impl HooksConfig {
    /// Merge two configs: `self` runs first (higher priority).
    pub fn merge(mut self, other: HooksConfig) -> HooksConfig {
        self.pre_tool_use.extend(other.pre_tool_use);
        self.post_tool_use.extend(other.post_tool_use);
        self.post_tool_use_failure
            .extend(other.post_tool_use_failure);
        self.permission_request.extend(other.permission_request);
        self.user_prompt_submit.extend(other.user_prompt_submit);
        self.stop.extend(other.stop);
        self.subagent_stop.extend(other.subagent_stop);
        self.session_start.extend(other.session_start);
        self.session_end.extend(other.session_end);
        self.notification.extend(other.notification);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.post_tool_use_failure.is_empty()
            && self.permission_request.is_empty()
            && self.user_prompt_submit.is_empty()
            && self.stop.is_empty()
            && self.subagent_stop.is_empty()
            && self.session_start.is_empty()
            && self.session_end.is_empty()
            && self.notification.is_empty()
    }
}

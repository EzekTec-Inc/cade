/// Capability-aware tool catalog.
///
/// Wraps the existing `all_meta_schemas()` and `schemas_for_toolset()` functions
/// and filters them based on the active `CapabilitySet`.
use cade_core::capabilities::{Capability, CapabilitySet};
use cade_core::toolsets::Toolset;
use serde_json::Value;

use super::manager::schemas_for_toolset;
use super::meta::all_meta_schemas;

/// Returns the tool name from a JSON schema object.
fn tool_name(schema: &Value) -> &str {
    schema["name"].as_str().unwrap_or("")
}

/// Classify a meta-tool schema into its required capability.
/// Returns `None` for core tools that are always available.
fn meta_tool_capability(name: &str) -> Option<Capability> {
    match name {
        // Agentic pack
        "run_subagent" | "list_agents" | "message_agent" | "reflect" => Some(Capability::Agentic),
        "store_artifact" => Some(Capability::Agentic),

        // Advanced memory pack
        "update_memory_typed" | "link_memory_evidence" => Some(Capability::AdvancedMemory),

        // Web pack
        "web_search" | "fetch_doc" | "browser_screenshot" => Some(Capability::Web),


        // Core tools — always available
        // update_memory, memory_apply_patch, archival_memory_insert,
        // archival_memory_search, conversation_search, search_memory,
        // load_skill, install_skill, run_skill_script, load_skill_ref,
        // create_checkpoint, list_checkpoints, restore_checkpoint
        _ => None,
    }
}

/// Classify a native (non-meta) tool schema into its required capability.
/// Returns `None` for core tools.
fn native_tool_capability(name: &str) -> Option<Capability> {
    match name {
        "desktop_screenshot" | "desktop_list_windows" | "desktop_control" | "desktop_notify" => {
            Some(Capability::Desktop)
        }
        // All other native tools (bash, read_file, write_file, edit_file,
        // apply_patch, grep, glob, ask_user_question, plan tools) are core.
        _ => None,
    }
}

/// Filter meta-tool schemas to only those allowed by the capability set.
pub fn meta_schemas_for_capabilities(caps: &CapabilitySet) -> Vec<Value> {
    all_meta_schemas()
        .into_iter()
        .filter(|schema| {
            let name = tool_name(schema);
            match meta_tool_capability(name) {
                None => true, // core — always included
                Some(cap) => caps.is_enabled(cap),
            }
        })
        .collect()
}

/// Filter native-tool schemas (for a toolset) to only those allowed by capabilities.
pub fn native_schemas_for_capabilities(toolset: Toolset, caps: &CapabilitySet) -> Vec<Value> {
    schemas_for_toolset(toolset, false)
        .into_iter()
        .filter(|schema| {
            let name = tool_name(schema);
            match native_tool_capability(name) {
                None => true,
                Some(cap) => caps.is_enabled(cap),
            }
        })
        .collect()
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_caps_exclude_desktop_and_web() {
        let caps = CapabilitySet::core();
        let meta = meta_schemas_for_capabilities(&caps);
        let native = native_schemas_for_capabilities(Toolset::Default, &caps);

        let meta_names: Vec<&str> = meta.iter().map(tool_name).collect();
        let native_names: Vec<&str> = native.iter().map(tool_name).collect();

        // Core always has memory, checkpoint, skill tools
        assert!(meta_names.contains(&"update_memory"));
        assert!(meta_names.contains(&"create_checkpoint"));
        assert!(meta_names.contains(&"load_skill"));

        // Core excludes optional packs
        assert!(!meta_names.contains(&"web_search"));
        assert!(!meta_names.contains(&"run_subagent"));
        assert!(!native_names.contains(&"desktop_screenshot"));
    }

    #[test]
    fn full_caps_include_everything() {
        let caps = CapabilitySet::full();
        let meta = meta_schemas_for_capabilities(&caps);
        let native = native_schemas_for_capabilities(Toolset::Default, &caps);

        let meta_names: Vec<&str> = meta.iter().map(tool_name).collect();
        let native_names: Vec<&str> = native.iter().map(tool_name).collect();

        assert!(meta_names.contains(&"web_search"));
        assert!(meta_names.contains(&"run_subagent"));
        assert!(native_names.contains(&"desktop_screenshot"));
    }

    #[test]
    fn custom_caps_include_agentic_not_desktop() {
        let mut caps = CapabilitySet::core();
        caps.enable(Capability::Agentic);

        let meta = meta_schemas_for_capabilities(&caps);
        let native = native_schemas_for_capabilities(Toolset::Default, &caps);

        let meta_names: Vec<&str> = meta.iter().map(tool_name).collect();
        let native_names: Vec<&str> = native.iter().map(tool_name).collect();

        assert!(meta_names.contains(&"run_subagent"));
        assert!(!meta_names.contains(&"web_search"));
        assert!(!native_names.contains(&"desktop_screenshot"));
    }
}

// endregion: --- Tests

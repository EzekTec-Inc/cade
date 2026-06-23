/// Meta-tool registration: memory, skills, and subagent tools.
///
/// This module owns the JSON schemas and server-registration logic for all
/// meta tools.  Previously these lived in `src/main.rs`; centralising them
/// here lets `ToolRuntime` and `headless.rs` share a single source of truth.
use crate::agent::client::{CreateToolRequest, HttpTransport};
use serde_json::{Value, json};

// region:    --- Public API

/// Register all meta tools with the server and return their IDs.
/// Returns the list of IDs that were successfully registered/upserted.
pub async fn register_meta_tools(client: &HttpTransport) -> Vec<String> {
    let schemas = all_meta_schemas();
    let mut ids = Vec::new();
    for schema in schemas {
        let req = CreateToolRequest {
            source_code: String::new(),
            source_type: "json".to_string(),
            json_schema: Some(schema),
            tags: vec!["cade".to_string(), "meta".to_string()],
        };
        match client.create_tool(req).await {
            Ok(tool) => ids.push(tool.id),
            Err(e) => tracing::debug!("meta tool registration: {e}"),
        }
    }
    ids
}

/// All meta-tool JSON schemas in a single list.
pub fn all_meta_schemas() -> Vec<Value> {
    #[allow(unused_mut)] // mut needed when "web" feature is enabled
    let mut schemas = vec![
        schema_update_memory(),
        schema_memory_apply_patch(),
        schema_archival_memory_insert(),
        schema_archival_memory_search(),
        schema_conversation_search(),
        schema_query_event_log(),
        schema_search_memory(),
        schema_recall(),
        schema_load_skill(),
        schema_unload_skill(),
        schema_install_skill(),
        schema_install_plugin(),
        schema_run_skill_script(),
        schema_load_skill_ref(),
        schema_run_subagent(),
        schema_run_parallel_subagents(),
        schema_cancel_subagent(),
        schema_list_agents(),
        schema_message_agent(),
        schema_create_checkpoint(),
        schema_list_checkpoints(),
        schema_restore_checkpoint(),
        schema_store_artifact(),
        schema_update_memory_typed(),
        schema_link_memory_evidence(),
        schema_update_memory_field(),
        schema_reflect(),
        schema_answer(),
    ];

    // Phase 6: web tools (optional)
    #[cfg(feature = "web")]
    {
        schemas.push(cade_web::WebSearchTool::schema());
        schemas.push(cade_web::FetchDocTool::schema());
        schemas.push(cade_web::BrowserScreenshotTool::schema());
    }

    schemas
}

// endregion: --- Public API

// region:    --- Memory schemas

fn schema_update_memory() -> Value {
    json!({
        "name": "update_memory",
        "description": "Update or delete a persistent memory block. Use this to store important information about the user, project, or yourself that should be remembered across conversations. Call this whenever you learn something worth remembering.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block name: 'human' (user info), 'project' (project context), 'persona' (your identity/style), or any custom label"
                },
                "value": {
                    "type": "string",
                    "description": "Content to store in the memory block (required for set/append, ignore for delete)"
                },
                "operation": {
                    "type": "string",
                    "enum": ["set", "append", "delete"],
                    "description": "set = replace the block entirely, append = add to existing content, delete = remove the block"
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this block is for (optional, shown in /memory display)"
                }
            },
            "required": ["label"]
        }
    })
}

fn schema_memory_apply_patch() -> Value {
    json!({
        "name": "memory_apply_patch",
        "description": "Edit a persistent memory block using a unified diff patch. Use this to store important information about the user, project, or yourself that should be remembered across conversations.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block name: 'human' (user info), 'project' (project context), 'persona' (your identity/style), or any custom label"
                },
                "patch": {
                    "type": "string",
                    "description": "A valid unified diff patch string. To create a new block or replace entirely, write a patch from an empty file."
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this block is for (optional, shown in /memory display)"
                }
            },
            "required": ["label", "patch"]
        }
    })
}

fn schema_archival_memory_insert() -> Value {
    json!({
        "name": "archival_memory_insert",
        "description": "Store large text, logs, code snippets, or subagent outputs out-of-context. Use this so your active context window does not overflow.",
        "parameters": {
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "The large text to store" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for later retrieval"
                }
            },
            "required": ["content"]
        }
    })
}

fn schema_archival_memory_search() -> Value {
    json!({
        "name": "archival_memory_search",
        "description": "Search archival memory using FTS5 (BM25 ranking). Returns FTS snippets of the matched blocks.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search term or tag" },
                "limit": { "type": "integer", "description": "Max results to return (default 10)" }
            },
            "required": ["query"]
        }
    })
}

fn schema_conversation_search() -> Value {
    json!({
        "name": "conversation_search",
        "description": "Search past conversation history. Your active context window drops older messages. Use this tool to retrieve dropped dialogue — decisions made, errors seen, steps already completed.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword or phrase to search for in past messages" }
            },
            "required": ["query"]
        }
    })
}

fn schema_query_event_log() -> Value {
    json!({
        "name": "query_event_log",
        "description": "Query the exact, uncompressed history of your file edits, commands, and subagent executions. Retrieves exact historical data with zero compression loss.",
        "parameters": {
            "type": "object",
            "properties": {
                "keyword": { "type": "string", "description": "Keyword to search for in past events" },
                "limit": { "type": "integer", "description": "Max results to return (default 10)" }
            },
            "required": ["keyword"]
        }
    })
}

fn schema_search_memory() -> Value {
    json!({
        "name": "search_memory",
        "description": "Search your persistent memory blocks by keyword or memory type. Returns matching blocks with a contextual excerpt. Archived ('long-term') blocks that match are automatically promoted back to active memory so they reappear in your prompt. Use this whenever you need context that may have been archived.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword or phrase to search for across memory block labels and values" },
                "memory_type": { "type": "string", "description": "Optional. Filter results by a specific memory semantic type (e.g., 'decision', 'convention', 'procedural')" }
            },
            "required": ["query"]
        }
    })
}

// endregion: --- Memory schemas

fn schema_recall() -> Value {
    json!({
        "name": "recall",
        "description": "Search all memory sources at once: persistent memory blocks, conversation history, archival memory, and event log. Returns ranked results from all sources using reciprocal rank fusion. Use this as a single entry point when you need to find something but aren't sure where it was stored.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword or phrase to search for across all memory sources" },
                "limit": { "type": "integer", "description": "Max results to return (default 10)" }
            },
            "required": ["query"]
        }
    })
}

fn schema_answer() -> Value {
    json!({
        "name": "answer",
        "description": "Answer a question using your persistent memory — searches all memory sources (blocks, conversation history, archival) and synthesizes a concise, grounded answer. Use this when you need a direct answer from what you already know, rather than raw search results.",
        "parameters": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to answer from memory"
                },
                "memory_type": {
                    "type": "string",
                    "description": "Optional. Filter to a specific memory type (e.g. 'decision', 'fact', 'preference')"
                },
                "max_sources": {
                    "type": "integer",
                    "description": "Max memory sources to consider (default 5)"
                }
            },
            "required": ["question"]
        }
    })
}

// region:    --- Skill schemas

fn schema_load_skill() -> Value {
    json!({
        "name": "load_skill",
        "description": "Load the full content of a skill into context. Call this when starting a task that matches one of the available skills listed in your system prompt.",
        "parameters": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The skill ID to load (from the Available Skills list)"
                }
            },
            "required": ["id"]
        }
    })
}

fn schema_unload_skill() -> Value {
    json!({
        "name": "unload_skill",
        "description": "Unload a previously loaded skill from context to free up context window budget. The skill body will no longer be injected into the system prompt on the next turn. Use when a skill is no longer needed for the current task.",
        "parameters": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The skill ID to unload (must be currently loaded)"
                }
            },
            "required": ["id"]
        }
    })
}

fn schema_install_skill() -> Value {
    json!({
        "name": "install_skill",
        "description": "Download and install a skill from any URL that serves a SKILL.MD file. Accepts GitHub tree/blob URLs, bare GitHub repo URLs with a --skill selector (e.g. https://github.com/github/awesome-copilot), GitHub shorthand (owner/repo), skill registry URLs (e.g. https://agentskill.sh/@user/skill), or any direct URL to a SKILL.MD file. Use when the user asks to install a skill or pastes a skill install prompt.",
        "parameters": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to install the skill from — GitHub URL, GitHub shorthand (owner/repo), skill registry URL, or direct SKILL.MD URL"
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "Where to install: project (.cade/skills/) or global (~/.cade/skills/)"
                },
                "skill": {
                    "type": "string",
                    "description": "Name of a specific skill to install from a multi-skill repository (e.g. 'rust-mcp-server-generator'). Required when the URL points to a repo root rather than a specific skill directory."
                }
            },
            "required": ["url"]
        }
    })
}

fn schema_install_plugin() -> Value {
    json!({
        "name": "install_plugin",
        "description": "Download and install a plugin package (which can bundle skills, subagents, and MCP servers) from a central registry or direct URL.",
        "parameters": {
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to the plugin tarball or registry entry" },
                "plugin_id": { "type": "string", "description": "Unique ID of the plugin (e.g. '@author/name')" }
            },
            "required": ["url", "plugin_id"]
        }
    })
}

fn schema_run_skill_script() -> Value {
    json!({
        "name": "run_skill_script",
        "description": "Execute a script from a skill's scripts/ directory. Use after load_skill to run deterministic tooling bundled with the skill.",
        "parameters": {
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The skill ID that owns the script"
                },
                "script": {
                    "type": "string",
                    "description": "Script name (filename stem, e.g. 'explain_error')"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional arguments to pass to the script"
                }
            },
            "required": ["skill_id", "script"]
        }
    })
}

fn schema_load_skill_ref() -> Value {
    json!({
        "name": "load_skill_ref",
        "description": "Lazy-load a reference document from a skill's references/ directory. Use only when you need deep documentation to solve a specific problem — avoids injecting tokens unnecessarily.",
        "parameters": {
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The skill ID that owns the reference"
                },
                "doc": {
                    "type": "string",
                    "description": "Reference doc name (filename stem, e.g. 'dictionary_of_pain')"
                }
            },
            "required": ["skill_id", "doc"]
        }
    })
}

// endregion: --- Skill schemas

// region:    --- Subagent schema

fn schema_run_subagent() -> Value {
    json!({
        "name": "run_subagent",
        "description": "Spawn a subagent to handle a task autonomously. Only the final answer is returned — your context stays clean. Use 'claude-3-5-haiku-latest' or 'gemini-2.5-pro' as the model for fast, read-heavy tasks like codebase search, log summarization, or deep exploration.",
        "parameters": {
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "description": "Subagent mode: 'plan' (read-only) or 'build' (full access). Default is 'build'."
                },
                "prompt": {
                    "type": "string",
                    "description": "The task description for the subagent"
                },
                "background": {
                    "type": "boolean",
                    "description": "Run in background — tool returns immediately, you get notified on completion (default false)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional: deploy an existing stateful agent as the subagent by its agent ID"
                },
                "model": {
                    "type": "string",
                    "description": "Optional: override the subagent's model. Strongly recommended to specify a fast model like 'claude-3-5-haiku-latest' or 'gemini-2.5-pro' for simple tasks to increase speed."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional: a custom system prompt to uniquely skill the subagent for the task"
                },
                "description": {
                    "type": "string",
                    "description": "Optional: a short description of the subagent's role or purpose"
                },
                "max_tokens_budget": {
                    "type": "integer",
                    "description": "Optional: Maximum combined tokens (prompt + completion) this subagent is allowed to consume before being forcefully terminated."
                },
                "test_command": {
                    "type": "string",
                    "description": "Optional: a bash test command that the subagent MUST run and pass (e.g. 'cargo test'). The subagent's response will be rejected if it does not prove this test passed."
                },
                "human_review": {
                    "type": "boolean",
                    "description": "Optional: prompt the user to review, approve, or provide feedback to re-task the subagent before its output is accepted. Set to true for sensitive or highly destructive tasks."
                }
            },
            "required": ["prompt"]
        }
    })
}

fn schema_run_parallel_subagents() -> Value {
    json!({
        "name": "run_parallel_subagents",
        "description": "Spawn multiple subagents simultaneously to analyze different files or solve independent problems, then aggregate their results. You can either provide a list of 'tasks' OR provide a 'team_id' and a 'prompt' to dispatch the same prompt to all members of a team.",
        "parameters": {
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "List of subagent configuration objects. Each object must have a 'prompt' and optionally 'mode', 'model', 'system_prompt', 'description', 'test_command', 'max_tokens_budget'.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": { "type": "string" },
                            "mode": { "type": "string" },
                            "model": { "type": "string" },
                            "system_prompt": { "type": "string" },
                            "description": { "type": "string" },
                            "test_command": { "type": "string" },
                            "max_tokens_budget": { "type": "integer" }
                        },
                        "required": ["prompt"]
                    }
                },
                "team_id": {
                    "type": "string",
                    "description": "Optional: ID of the team to dispatch to (e.g. 'default'). If provided, 'prompt' must also be provided."
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional: The prompt to send to all team members when using 'team_id'."
                }
            },
            "required": []
        }
    })
}

fn schema_cancel_subagent() -> Value {
    json!({
        "name": "cancel_subagent",
        "description": "Cancel a running background subagent by its ID.",
        "parameters": {
            "type": "object",
            "properties": {
                "subagent_id": {
                    "type": "string",
                    "description": "The ID of the subagent to cancel"
                }
            },
            "required": ["subagent_id"]
        }
    })
}

fn schema_list_agents() -> Value {
    json!({
        "name": "list_agents",
        "description": "List all active agents on the server to discover colleagues you can collaborate with. Returns their names, IDs, and descriptions.",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    })
}

fn schema_message_agent() -> Value {
    json!({
        "name": "message_agent",
        "description": "Send a message to another named agent (colleague) and wait for their response. Useful for delegating specialized tasks or asking for review.",
        "parameters": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "The name or ID of the target agent"
                },
                "message": {
                    "type": "string",
                    "description": "The message/task description to send to the agent"
                }
            },
            "required": ["target", "message"]
        }
    })
}

// endregion: --- Subagent schema

// region:    --- Checkpoint / artifact schemas

fn schema_create_checkpoint() -> Value {
    json!({
        "name": "create_checkpoint",
        "description": "Create a checkpoint of the current working-tree state. Commits dirty git changes as a snapshot so you can restore to this exact state later. Use before risky operations, refactors, or long task sequences.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Short label for the checkpoint (e.g. 'before-refactor')"
                },
                "description": {
                    "type": "string",
                    "description": "Longer description of what this checkpoint captures"
                }
            },
            "required": []
        }
    })
}

fn schema_list_checkpoints() -> Value {
    json!({
        "name": "list_checkpoints",
        "description": "List all checkpoints for the current agent session, newest first.",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    })
}

fn schema_restore_checkpoint() -> Value {
    json!({
        "name": "restore_checkpoint",
        "description": "Restore the working tree to a previously created checkpoint. Hard resets the git tree to the commit captured at that point and marks the checkpoint as current.",
        "parameters": {
            "type": "object",
            "properties": {
                "checkpoint_id": {
                    "type": "string",
                    "description": "The checkpoint ID to restore (from list_checkpoints)"
                }
            },
            "required": ["checkpoint_id"]
        }
    })
}

fn schema_store_artifact() -> Value {
    json!({
        "name": "store_artifact",
        "description": "Persist a text artifact (log output, diff, test report, fetched document) for later retrieval. Returns an artifact ID.",
        "parameters": {
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["log", "diff", "test_report", "fetched_doc", "screenshot", "trace", "other"],
                    "description": "Type of artifact"
                },
                "content": {
                    "type": "string",
                    "description": "The text content to store"
                },
                "label": {
                    "type": "string",
                    "description": "Optional short label for the artifact"
                }
            },
            "required": ["kind", "content"]
        }
    })
}

// endregion: --- Checkpoint / artifact schemas

// region:    --- Typed memory / provenance / reflection schemas

fn schema_update_memory_typed() -> Value {
    json!({
        "name": "update_memory_typed",
        "description": "Store a typed fact in persistent memory with schema validation. Use specific types for better organisation and provenance tracking. Prefer this over update_memory when you want to categorise the information.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block name (e.g. 'api_design_decision', 'db_schema_fact')"
                },
                "value": {
                    "type": "string",
                    "description": "Content to store"
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["project_fact", "user_pref", "decision", "constraint", "convention",
                             "dependency", "person", "environment", "generic"],
                    "description": "Semantic type of this memory block"
                },
                "confidence": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Confidence 0–1 (default 1.0 = certain)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for grouping"
                }
            },
            "required": ["label", "value", "memory_type"]
        }
    })
}

fn schema_update_memory_field() -> Value {
    json!({
        "name": "update_memory_field",
        "description": "Patch a single field of a structured (JSON) memory block at a JSON-pointer path without re-emitting the entire body. The block body must already be valid JSON; if not, use update_memory(set,...) first to seed structure.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block name (e.g. 'active_goal', 'project', 'working_set')"
                },
                "path": {
                    "type": "string",
                    "description": "JSON pointer (RFC 6901), e.g. '/next_steps/0' or '/status'"
                },
                "op": {
                    "type": "string",
                    "enum": ["set", "append", "remove"],
                    "description": "set = replace/create at path, append = push onto array at path, remove = delete key/index at path"
                },
                "value": {
                    "description": "JSON value to set or append (required for set/append, ignored for remove)"
                }
            },
            "required": ["label", "path", "op"]
        }
    })
}

fn schema_link_memory_evidence() -> Value {
    json!({
        "name": "link_memory_evidence",
        "description": "Link a piece of evidence (a message, file, or tool result) to a memory block to explain why it was recorded. Builds a traceable provenance chain.",
        "parameters": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Memory block to annotate"
                },
                "kind": {
                    "type": "string",
                    "enum": ["message", "tool_result", "file", "user_assertion"],
                    "description": "Type of evidence source"
                },
                "reference": {
                    "type": "string",
                    "description": "Message ID, file path, or tool call ID that supports this memory"
                },
                "excerpt": {
                    "type": "string",
                    "description": "A brief quote or summary from the source"
                }
            },
            "required": ["label", "kind", "reference"]
        }
    })
}

fn schema_reflect() -> Value {
    json!({
        "name": "reflect",
        "description": "Trigger a reflection pass over recent conversation history to extract and update memory blocks. Use after completing a significant coding task to capture what was learned.",
        "parameters": {
            "type": "object",
            "properties": {
                "focus": {
                    "type": "string",
                    "description": "Optional: what aspect to focus reflection on (e.g. 'project conventions', 'user preferences')"
                }
            },
            "required": []
        }
    })
}

// endregion: --- Typed memory / provenance / reflection schemas

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_meta_schemas_have_name_and_parameters() {
        // -- Exec
        let schemas = all_meta_schemas();

        // -- Check
        assert!(!schemas.is_empty(), "should have meta schemas");
        for schema in &schemas {
            assert!(
                schema["name"].as_str().is_some(),
                "schema missing 'name': {schema}"
            );
            assert!(
                !schema["parameters"].is_null(),
                "schema '{}' missing 'parameters'",
                schema["name"].as_str().unwrap_or("?")
            );
        }
    }

    #[test]
    fn test_schema_names_match_tool_ids() {
        use cade_core::tool_ids::*;
        // -- Exec
        let schemas = all_meta_schemas();
        let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();

        // -- Check — every canonical meta tool ID should be present
        for id in META_TOOL_IDS {
            assert!(names.contains(id), "missing schema for '{id}'");
        }
    }
}

// endregion: --- Tests

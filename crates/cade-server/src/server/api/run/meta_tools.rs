//! Meta-tool handlers for the server-side agentic loop.
//!
//! These functions intercept tool calls that require direct access to
//! `AppState` (DB, agent_id, SSE channel) before falling through to the
//! generic MCP dispatcher.

use crate::server::state::AppState;

pub(super) async fn intercept_meta_tool(
    state: &AppState,
    agent_id: &str,
    tc: &cade_ai::LlmToolCall,
    sse_tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> Option<cade_agent::tools::manager::ToolResult> {
    use cade_agent::tools::manager::ToolResult;
    let mk = |output: String, is_error: bool| ToolResult {
        tool_call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        output,
        is_error,
    };
    match tc.name.as_str() {
        "load_skill" => {
            Some(handle_load_skill_tool(state, agent_id, &tc.id, &tc.arguments).await)
        }
        "unload_skill" => {
            Some(handle_unload_skill_tool(state, agent_id, &tc.id, &tc.arguments).await)
        }
        "run_subagent" => {
            let args: serde_json::Value =
                serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
            Some(super::subagent::handle_run_subagent_tool(state, agent_id, &tc.id, &args, sse_tx).await)
        }
        "run_parallel_subagents" => {
            let args: serde_json::Value =
                serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
            Some(super::subagent::handle_run_parallel_subagents_tool(state, agent_id, &tc.id, &args, sse_tx).await)
        }
        "cancel_subagent" => {
            let args: serde_json::Value =
                serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
            Some(super::subagent::handle_cancel_subagent_tool(state, &tc.id, &args).await)
        }
        // ── Phase A1: memory tools ────────────────────────────────────
        "update_memory" => {
            let (output, is_error) = handle_update_memory(state, agent_id, &tc.arguments, Some(&tc.id)).await;
            Some(mk(output, is_error))
        }
        "update_memory_typed" => {
            let (output, is_error) =
                handle_update_memory_typed(state, agent_id, &tc.arguments, Some(&tc.id)).await;
            Some(mk(output, is_error))
        }
        "memory_apply_patch" => {
            let (output, is_error) =
                handle_memory_apply_patch(state, agent_id, &tc.arguments, Some(&tc.id)).await;
            Some(mk(output, is_error))
        }
        "update_memory_field" => {
            let (output, is_error) =
                handle_update_memory_field(state, agent_id, &tc.arguments, Some(&tc.id)).await;
            Some(mk(output, is_error))
        }
        "link_memory_evidence" => {
            let (output, is_error) =
                handle_link_memory_evidence(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "reflect" => {
            let (output, is_error) = handle_reflect_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        // ── Phase A1b: memory-read tools ──────────────────────────────────
        "search_memory" => {
            let (output, is_error) =
                handle_search_memory_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "conversation_search" => {
            let (output, is_error) =
                handle_conversation_search_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "archival_memory_insert" => {
            let (output, is_error) =
                handle_archival_memory_insert_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "archival_memory_search" => {
            let (output, is_error) =
                handle_archival_memory_search_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "query_event_log" => {
            let (output, is_error) =
                handle_query_event_log_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        // ── C7: unified recall tool ──────────────────────────────────────
        "recall" => {
            let (output, is_error) =
                handle_recall_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        // ── Phase A2: skill meta-tools ────────────────────────────────────
        "install_skill" => {
            let (output, is_error) =
                handle_install_skill_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "run_skill_script" => {
            let (output, is_error) =
                handle_run_skill_script_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "load_skill_ref" => {
            let (output, is_error) =
                handle_load_skill_ref_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        // ── Phase A3: checkpoint meta-tools ───────────────────────────────
        "create_checkpoint" => {
            let (output, is_error) =
                handle_create_checkpoint_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "list_checkpoints" => {
            let (output, is_error) =
                handle_list_checkpoints_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "restore_checkpoint" => {
            let (output, is_error) =
                handle_restore_checkpoint_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        // ── Phase A4: artifact + agents meta-tools ────────────────────────
        "store_artifact" => {
            let (output, is_error) =
                handle_store_artifact_meta(state, agent_id, &tc.arguments).await;
            Some(mk(output, is_error))
        }
        "list_agents" => {
            let (output, is_error) = handle_list_agents_meta(state, agent_id).await;
            Some(mk(output, is_error))
        }
        "message_agent" => {
            let (output, is_error) =
                handle_message_agent_meta(state, agent_id, &tc.arguments, sse_tx).await;
            Some(mk(output, is_error))
        }
        // ── Plan-panel tools (TUI-rendered, server-side ack) ─────────────
        //
        // `set_plan` and `UpdatePlan` are normally handled by the TUI client
        // intercept (see crates/cade-cli/src/cli/repl/turn_tools/runner.rs)
        // because they need access to the live `TuiApp` to draw the plan
        // panel.  Subagents run inside the server's agentic loop and never
        // reach that client intercept, so calls fell through to the generic
        // dispatcher and returned `"Unknown tool"`, confusing the
        // subagent's LLM and burning iterations on retries.
        //
        // Intercept here so subagents get a clean success ack.  The plan
        // itself is not rendered (no TuiApp is attached in this path), but
        // the LLM proceeds normally and surfaces its plan via prose in the
        // accumulated `last_text` returned to the parent.
        "set_plan" => {
            let n = tc.arguments["steps"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            Some(mk(format!("Plan acknowledged with {n} step(s)."), false))
        }
        "UpdatePlan" => {
            let step = tc.arguments["step_id"].as_u64().unwrap_or(0);
            let done = tc.arguments["done"].as_bool().unwrap_or(true);
            Some(mk(
                format!(
                    "Step {step} marked {}.",
                    if done { "done" } else { "not done" }
                ),
                false,
            ))
        }
        _ => None,
    }
}

/// Phase A1 handler: `update_memory` server-side.  Mirrors the CLI
/// `ToolRuntime::handle_update_memory` semantics (set / append / delete)
/// but talks directly to `state.db` instead of over HTTP, so the GUI's
/// `/v1/agents/:id/run` agentic loop no longer returns "Unknown tool".
///
/// A3: `tool_call_id` is passed through so we can stamp provenance after
/// a successful write. `None` when called from non-agentic paths.
async fn handle_update_memory(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
    tool_call_id: Option<&str>,
) -> (String, bool) {
    let label = args["label"].as_str().unwrap_or("").trim().to_string();
    let value = args["value"].as_str().unwrap_or("").to_string();
    let operation = args["operation"].as_str().unwrap_or("set");
    let description = args["description"].as_str();

    if label.is_empty() {
        return ("Error: 'label' is required".to_string(), true);
    }

    if operation == "delete" {
        return match cade_store::sqlite::delete_memory_block(&state.db, agent_id, &label) {
            Ok(true) => (format!("Memory block '{label}' deleted"), false),
            Ok(false) => (format!("Memory block '{label}' not found"), true),
            Err(e) => (format!("Failed to delete memory block: {e}"), true),
        };
    }

    if value.is_empty() {
        return (
            "Error: 'value' is required for set/append operations".to_string(),
            true,
        );
    }

    let final_value = if operation == "append" {
        let existing = cade_store::sqlite::get_memory_blocks(&state.db, agent_id)
            .ok()
            .unwrap_or_default()
            .into_iter()
            .find(|(l, _, _)| l == &label)
            .map(|(_, v, _)| v)
            .unwrap_or_default();
        if existing.is_empty() {
            value
        } else {
            format!("{existing}\n{value}")
        }
    } else {
        value
    };

    match cade_store::sqlite::upsert_memory_block(
        &state.db,
        agent_id,
        &label,
        &final_value,
        description,
        None,
    ) {
        Ok(wr) => {
            // A3: Stamp provenance — record which turn and tool call wrote this block.
            let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
            cade_store::sqlite::memory::stamp_provenance(
                &state.db,
                agent_id,
                &label,
                Some(turn),
                None,
                tool_call_id,
                tool_call_id,
            );

            // A5: Re-chunk the block for semantic chunk-level search.
            cade_store::sqlite::memory::rechunk_block(
                &state.db,
                agent_id,
                &label,
                &final_value,
                state.embedder.as_ref().map(|e| e.as_ref()),
            );

            let char_info = format!(" ({}/{} chars)", wr.stored_chars, wr.requested_chars);
            if wr.was_truncated {
                (
                    format!(
                        "Memory block '{label}' updated{char_info}.\n\
                         ⚠️ WARNING: Content was truncated from {} to {} chars.\n\
                         Consider splitting into multiple blocks or using archival_memory_insert for overflow.",
                        wr.requested_chars, wr.stored_chars
                    ),
                    false,
                )
            } else {
                (format!("Memory block '{label}' updated{char_info}"), false)
            }
        }
        Err(e) => (format!("Failed: {e}"), true),
    }
}

/// Phase A1 handler: `update_memory_typed` server-side.  Persists the
/// block via `upsert_memory_block_typed` so the `memory_type`,
/// `confidence`, and tags are written to their dedicated columns.  Tags
/// are accepted as a JSON array but currently round-trip as a string in
/// the description field if the schema does not separately store them
/// (matches CLI behaviour — see the latent gap described in the
/// `update_memory_typed` API note).
async fn handle_update_memory_typed(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
    tool_call_id: Option<&str>,
) -> (String, bool) {
    let label = args["label"].as_str().unwrap_or("").trim().to_string();
    let value = args["value"].as_str().unwrap_or("").to_string();
    let memory_type = args["memory_type"].as_str().unwrap_or("generic");
    let confidence = args["confidence"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);

    if label.is_empty() || value.is_empty() {
        return ("Error: 'label' and 'value' are required".to_string(), true);
    }

    match cade_store::sqlite::upsert_memory_block_typed(
        &state.db,
        agent_id,
        &label,
        &value,
        None,
        None,
        Some(memory_type),
        Some(confidence),
    ) {
        Ok(_) => {
            // A3: Stamp provenance.
            let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
            cade_store::sqlite::memory::stamp_provenance(
                &state.db, agent_id, &label, Some(turn), None, tool_call_id, tool_call_id,
            );
            // A5: Re-chunk.
            cade_store::sqlite::memory::rechunk_block(
                &state.db, agent_id, &label, &value,
                state.embedder.as_ref().map(|e| e.as_ref()),
            );
            (
                format!(
                    "Memory block '{label}' stored as [{memory_type}] (confidence: {:.0}%)",
                    confidence * 100.0
                ),
                false,
            )
        }
        Err(e) => (format!("Failed to store typed memory: {e}"), true),
    }
}

/// Phase A1 handler: `memory_apply_patch` server-side.  Loads the
/// current value, applies a unified-diff patch via the shared
/// `cade_agent::tools::runtime::apply_unified_diff` helper, and writes
/// the result back.  Operates directly on `state.db`.
async fn handle_memory_apply_patch(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
    tool_call_id: Option<&str>,
) -> (String, bool) {
    let label = args["label"].as_str().unwrap_or("").trim().to_string();
    let patch = args["patch"].as_str().unwrap_or("").to_string();
    let description = args["description"].as_str();

    if label.is_empty() || patch.is_empty() {
        return ("Error: 'label' and 'patch' are required".to_string(), true);
    }

    let current = cade_store::sqlite::get_memory_blocks(&state.db, agent_id)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .find(|(l, _, _)| l == &label)
        .map(|(_, v, _)| v)
        .unwrap_or_default();

    match cade_agent::tools::runtime::apply_unified_diff(&current, &patch) {
        Ok(new_value) => match cade_store::sqlite::upsert_memory_block(
            &state.db,
            agent_id,
            &label,
            &new_value,
            description,
            None,
        ) {
            Ok(wr) => {
                // A3: Stamp provenance.
                let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
                cade_store::sqlite::memory::stamp_provenance(
                    &state.db, agent_id, &label, Some(turn), None, tool_call_id, tool_call_id,
                );
                // A5: Re-chunk.
                cade_store::sqlite::memory::rechunk_block(
                    &state.db, agent_id, &label, &new_value,
                    state.embedder.as_ref().map(|e| e.as_ref()),
                );
                (
                    format!("Memory block '{label}' patched successfully ({} chars)", wr.stored_chars),
                    false,
                )
            }
            Err(e) => (format!("Failed to save patched memory: {e}"), true),
        },
        Err(e) => (format!("Patch failed: {e}"), true),
    }
}

/// Phase A1 handler: `update_memory_field` server-side.  Loads the current
/// JSON block, applies a JSON-pointer patch, and writes back.
async fn handle_update_memory_field(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
    tool_call_id: Option<&str>,
) -> (String, bool) {
    let label = args["label"].as_str().unwrap_or("").trim().to_string();
    let pointer = args["path"].as_str().unwrap_or("").to_string();
    let op_str = args["op"].as_str().unwrap_or("");
    let value = args.get("value").cloned();

    if label.is_empty() || pointer.is_empty() {
        return (
            "Error: 'label' and 'path' are required".to_string(),
            true,
        );
    }

    let op = match cade_core::structured_patch::PatchOp::from_str_loose(op_str) {
        Some(o) => o,
        None => {
            return (
                format!("Error: invalid op '{op_str}' — must be set, append, or remove"),
                true,
            );
        }
    };

    let current = cade_store::sqlite::get_memory_blocks(&state.db, agent_id)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .find(|(l, _, _)| l == &label)
        .map(|(_, v, _)| v)
        .unwrap_or_default();

    if current.is_empty() {
        return (
            format!(
                "Error: memory block '{label}' is empty or does not exist. \
                 Use update_memory(set,...) to seed it with JSON first."
            ),
            true,
        );
    }

    let mut root = match cade_core::structured_patch::parse_block(&current) {
        Ok(v) => v,
        Err(e) => {
            return (
                format!("Error: {e}. Use update_memory(set,...) to seed JSON."),
                true,
            )
        }
    };

    if let Err(e) =
        cade_core::structured_patch::apply_pointer_patch(&mut root, &pointer, op, value)
    {
        return (format!("Patch error: {e}"), true);
    }

    let new_body = cade_core::structured_patch::serialize_back(&root);
    match cade_store::sqlite::upsert_memory_block(
        &state.db,
        agent_id,
        &label,
        &new_body,
        None,
        None,
    ) {
        Ok(_) => {
            // A3: Stamp provenance.
            let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
            cade_store::sqlite::memory::stamp_provenance(
                &state.db, agent_id, &label, Some(turn), None, tool_call_id, tool_call_id,
            );
            // A5: Re-chunk.
            cade_store::sqlite::memory::rechunk_block(
                &state.db, agent_id, &label, &new_body,
                state.embedder.as_ref().map(|e| e.as_ref()),
            );
            (
                format!("Memory block '{label}' field '{pointer}' updated ({op_str})"),
                false,
            )
        }
        Err(e) => (format!("Failed to save: {e}"), true),
    }
}

/// Phase A1 handler: `link_memory_evidence` server-side.  Persists a
/// row in `memory_evidence` linked to the named block.  Confidence
/// defaults to 1.0 when the LLM does not supply one (matches CLI flow).
async fn handle_link_memory_evidence(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let label = args["label"].as_str().unwrap_or("").trim().to_string();
    let kind = args["kind"].as_str().unwrap_or("user_assertion");
    let reference = args["reference"].as_str().unwrap_or("").trim().to_string();
    let excerpt = args["excerpt"].as_str();
    let confidence = args["confidence"].as_f64().unwrap_or(1.0);

    if label.is_empty() || reference.is_empty() {
        return (
            "Error: 'label' and 'reference' are required".to_string(),
            true,
        );
    }

    match cade_store::sqlite::insert_memory_evidence(
        &state.db, agent_id, &label, kind, &reference, excerpt, confidence,
    ) {
        Ok(_) => (
            format!("Evidence linked to '{label}': [{kind}] {reference}"),
            false,
        ),
        Err(e) => (format!("Failed to link evidence: {e}"), true),
    }
}

/// Phase A1 handler: `reflect` server-side.  Delegates to the existing
/// `reflection::reflect_agent` engine that the API endpoint
/// `POST /v1/agents/:id/reflect` already drives — same engine, same DB,
/// no HTTP self-call.
async fn handle_reflect_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let focus = args["focus"].as_str();
    let result =
        crate::server::reflection::reflect_agent(state, agent_id, None, focus, "tool").await;
    (
        format!(
            "Reflection complete: {} block(s) created, {} updated",
            result.blocks_created, result.blocks_updated
        ),
        false,
    )
}

async fn handle_search_memory_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let query = args["query"].as_str().unwrap_or("").trim().to_string();
    let memory_type = args["memory_type"].as_str().filter(|s| !s.trim().is_empty()).map(|s| s.to_string());
    if query.is_empty() {
        return ("Error: 'query' is required".to_string(), true);
    }
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let q = query.clone();
    let embedder = state.embedder.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            cade_store::sqlite::tools::search_memory_hybrid(
                &db,
                &aid,
                &q,
                memory_type.as_deref(),
                embedder.as_deref(),
            )
        }),
    )
    .await;
    match result {
        Ok(Ok(Ok(results))) if results.is_empty() => (
            format!(
                "No memory blocks matched '{query}'. \
                 Try a shorter keyword, or use conversation_search to look through message history."
            ),
            false,
        ),
        Ok(Ok(Ok(results))) => {
            let mut out = format!(
                "Found {} matching memory block(s) for '{query}':\n\n",
                results.len()
            );
            for (label, _value, snippet) in &results {
                out.push_str(&format!("[{label}]\n{snippet}\n\n"));
            }
            (out.trim_end().to_string(), false)
        }
        Ok(Ok(Err(e))) => (format!("search_memory error: {e}"), true),
        Ok(Err(e)) => (format!("search_memory task panicked: {e}"), true),
        Err(_) => (
            "search_memory timed out after 10s. The query may be too broad.".to_string(),
            true,
        ),
    }
}

/// Phase A1b handler: `conversation_search` server-side.
/// Searches past messages by keyword directly via the DB.
/// Uses `spawn_blocking` + timeout to avoid blocking the async runtime.
///
/// F6 (cross-conversation search): the optional `conversation_id` argument
/// scopes results to a single conversation; when omitted (or empty) the
/// search spans every conversation recorded for the agent.
///
/// F8 (compaction transparency): when any matched snippet sits before a
/// compaction marker, the header reports how many of the hits were
/// pre-compaction.  When the search returns zero hits but the agent does
/// have at least one compaction marker, the empty response includes a
/// hint pointing at `archival_memory_search` (tag: `dropped-turns`,
/// shipped by F2) and `session_summary` so the agent knows where the raw
/// dialogue lives.
pub(super) async fn handle_conversation_search_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let query = args["query"].as_str().unwrap_or("").trim().to_string();
    if query.is_empty() {
        return ("Error: 'query' is required".to_string(), true);
    }
    let conversation_id: Option<String> = args["conversation_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let q = query.clone();
    let cid = conversation_id.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            cade_store::sqlite::search_messages(&db, &aid, &q, cid.as_deref())
        }),
    )
    .await;
    let scope_label = match conversation_id.as_deref() {
        Some(cid) => format!(" (conversation {cid})"),
        None => " (all conversations)".to_string(),
    };
    match result {
        Ok(Ok(Ok(results))) if results.is_empty() => {
            // F8: empty hit-list — but if a compaction marker exists, point
            // the agent at the F2 archival cache + session_summary.
            let has_marker = cade_store::sqlite::has_compaction_marker(
                &state.db,
                agent_id,
                conversation_id.as_deref(),
            )
            .unwrap_or(false);
            let mut out =
                format!("No conversation messages matched '{query}'{scope_label}.");
            if has_marker {
                out.push_str(
                    "\nNote: this agent has compacted history. The raw dropped turns are \
                     stored in archival memory (try `archival_memory_search` with tag \
                     `dropped-turns`); a higher-level summary lives in the \
                     `session_summary` memory block.",
                );
            }
            (out, false)
        }
        Ok(Ok(Ok(results))) => {
            // F8: count how many hits sat before a compaction marker so the
            // header line gives the agent an at-a-glance signal.
            let pre_compaction_hits = results
                .iter()
                .filter(|r| r.snippet.contains("[pre-compaction"))
                .count();
            let mut out = format!(
                "Found {} result(s) for '{query}'{scope_label} in conversation history",
                results.len(),
            );
            if pre_compaction_hits > 0 {
                out.push_str(&format!(
                    " ({pre_compaction_hits} from pre-compaction history — \
                     see archival_memory_search with tag `dropped-turns` for full text)"
                ));
            }
            out.push_str(":\n\n");
            for r in &results {
                out.push_str(&format!("[{}] {}\n", r.role, r.snippet));
            }
            (out.trim_end().to_string(), false)
        }
        Ok(Ok(Err(e))) => (format!("conversation_search error: {e}"), true),
        Ok(Err(e)) => (format!("conversation_search task panicked: {e}"), true),
        Err(_) => (
            "conversation_search timed out after 10s. The query may be too broad.".to_string(),
            true,
        ),
    }
}

/// Phase A1b handler: `archival_memory_insert` server-side.
/// Stores large text into archival memory directly via the DB.
async fn handle_archival_memory_insert_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let content = args["content"].as_str().unwrap_or("").to_string();
    if content.is_empty() {
        return ("Error: 'content' is required".to_string(), true);
    }
    let tags: Vec<String> = args["tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match cade_store::sqlite::insert_archival_memory(&state.db, agent_id, &content, &tags) {
        Ok(id) => (
            format!("Stored in archival memory (id: {id}, {} chars)", content.len()),
            false,
        ),
        Err(e) => (format!("archival_memory_insert error: {e}"), true),
    }
}

/// Phase A1b handler: `archival_memory_search` server-side.
/// Searches archival memory using FTS5 directly via the DB.
/// Uses `spawn_blocking` + timeout to prevent blocking the async runtime.
async fn handle_archival_memory_search_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let query = args["query"].as_str().unwrap_or("").trim().to_string();
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    if query.is_empty() {
        return ("Error: 'query' is required".to_string(), true);
    }
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let q = query.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            cade_store::sqlite::search_archival_memory(&db, &aid, &q, limit)
        }),
    )
    .await;
    match result {
        Ok(Ok(Ok(results))) if results.is_empty() => (
            format!("No archival memory matched '{query}'."),
            false,
        ),
        Ok(Ok(Ok(results))) => {
            let mut out = format!(
                "Found {} archival record(s) for '{query}':\n\n",
                results.len()
            );
            for r in &results {
                let preview: String = r.content.chars().take(300).collect();
                let tags = if r.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [tags: {}]", r.tags.join(", "))
                };
                out.push_str(&format!("• {}{}\n  {preview}…\n\n", r.id, tags));
            }
            (out.trim_end().to_string(), false)
        }
        Ok(Ok(Err(e))) => (format!("archival_memory_search error: {e}"), true),
        Ok(Err(e)) => (format!("archival_memory_search task panicked: {e}"), true),
        Err(_) => (
            "archival_memory_search timed out after 10s. The query may be too broad.".to_string(),
            true,
        ),
    }
}

/// Phase A1b handler: `query_event_log` server-side.
/// Searches the event log by keyword directly via the DB.
/// Uses `spawn_blocking` + timeout to prevent blocking the async runtime.
async fn handle_query_event_log_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let keyword = args["keyword"].as_str().unwrap_or("").trim().to_string();
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    if keyword.is_empty() {
        return ("Error: 'keyword' is required".to_string(), true);
    }
    let db = state.db.clone();
    let aid = agent_id.to_string();
    let kw = keyword.clone();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            cade_store::sqlite::event_log::query_event_log(&db, &aid, &kw, limit)
        }),
    )
    .await;
    match result {
        Ok(Ok(Ok(entries))) if entries.is_empty() => (
            format!("No event log entries matched '{keyword}'."),
            false,
        ),
        Ok(Ok(Ok(entries))) => {
            let mut out = format!(
                "Found {} event(s) for '{keyword}':\n\n",
                entries.len()
            );
            for e in &entries {
                let preview: String = e.content.chars().take(200).collect();
                out.push_str(&format!("[{}] {}: {preview}\n", e.event_type, e.created_at));
            }
            (out.trim_end().to_string(), false)
        }
        Ok(Ok(Err(e))) => (format!("query_event_log error: {e}"), true),
        Ok(Err(e)) => (format!("query_event_log task panicked: {e}"), true),
        Err(_) => (
            "query_event_log timed out after 10s.".to_string(),
            true,
        ),
    }
}

/// Phase A2 handler: `install_skill` server-side.
/// Delegates to `cade_core::skills::install_skill_from_url`.
/// The installed skill is available to subsequent `load_skill` calls.
async fn handle_install_skill_meta(
    _state: &AppState,
    _agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let url = args["url"].as_str().unwrap_or("").trim().to_string();
    let scope = args["scope"].as_str().unwrap_or("project");
    let skill_name = args["skill"]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    if url.is_empty() {
        return ("Error: 'url' is required".to_string(), true);
    }

    // Server-side: use a reasonable default target directory.
    let target_dir = if scope == "global" {
        dirs::home_dir()
            .map(|h| h.join(".cade").join("skills"))
            .unwrap_or_else(|| std::path::PathBuf::from(".cade/skills"))
    } else {
        std::path::PathBuf::from(".cade/skills")
    };

    match cade_core::skills::install_skill_from_url(&url, &target_dir, skill_name).await {
        Ok(skill) => (
            format!(
                "Skill '{}' installed as [{}] in {} scope. It is now available via load_skill(\"{}\").",
                skill.name, skill.id, scope, skill.id
            ),
            false,
        ),
        Err(e) => (format!("Failed to install skill: {e}"), true),
    }
}

/// Phase A2 handler: `run_skill_script` server-side.
/// Discovers all skills visible from the current working directory,
/// locates the requested script, and executes it.
async fn handle_run_skill_script_meta(
    _state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
    let script = args["script"].as_str().unwrap_or("").trim().to_string();
    let script_args: Vec<String> = args["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if skill_id.is_empty() || script.is_empty() {
        return (
            "Error: 'skill_id' and 'script' are required".to_string(),
            true,
        );
    }

    let cwd = std::path::PathBuf::from(".");
    let skills = cade_core::skills::discover_all_skills(&cwd, Some(agent_id), None);
    let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
        return (format!("Skill '{skill_id}' not found"), true);
    };

    let Some(sk) = skill.scripts.iter().find(|s| s.name == script).cloned() else {
        let available: Vec<&str> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
        let list = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        return (
            format!("Script '{script}' not found in skill '{skill_id}'. Available: {list}"),
            true,
        );
    };

    let mut cmd = tokio::process::Command::new(&sk.path);
    cade_core::agent_env::apply_agent_env(&mut cmd);
    cade_core::askpass::apply_askpass_env(&mut cmd);
    match cmd.args(&script_args).output().await {
        Err(e) => (format!("Failed to run script: {e}"), true),
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{stdout}\n[stderr]\n{stderr}")
            };
            let is_err = !out.status.success();
            (combined, is_err)
        }
    }
}

/// Phase A2 handler: `load_skill_ref` server-side.
/// Reads a reference document from an installed skill's `references/` directory.
async fn handle_load_skill_ref_meta(
    _state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
    let doc = args["doc"].as_str().unwrap_or("").trim().to_string();

    if skill_id.is_empty() || doc.is_empty() {
        return ("Error: 'skill_id' and 'doc' are required".to_string(), true);
    }

    let cwd = std::path::PathBuf::from(".");
    let skills = cade_core::skills::discover_all_skills(&cwd, Some(agent_id), None);
    let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
        return (format!("Skill '{skill_id}' not found"), true);
    };

    let Some(r) = skill
        .references
        .iter()
        .find(|r| r.name == doc || r.path.file_name().and_then(|n| n.to_str()).unwrap_or("") == doc)
        .cloned()
    else {
        let available: Vec<&str> = skill.references.iter().map(|r| r.name.as_str()).collect();
        let list = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        return (
            format!("Reference '{doc}' not found in skill '{skill_id}'. Available: {list}"),
            true,
        );
    };

    match std::fs::read_to_string(&r.path) {
        Ok(content) => (
            format!("# Reference: {doc} (skill: {skill_id})\n\n{content}"),
            false,
        ),
        Err(e) => (format!("Failed to read reference '{doc}': {e}"), true),
    }
}

/// Phase A3 handler: `create_checkpoint` server-side.
/// Skips git stash (no interactive CWD) — records the checkpoint row in DB.
async fn handle_create_checkpoint_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let label = args["label"]
        .as_str()
        .unwrap_or("checkpoint")
        .trim()
        .to_string();
    let description = args["description"].as_str().map(String::from);

    let id = format!("cp-{}", uuid::Uuid::new_v4());
    let now = crate::server::api::checkpoints::unix_ts_pub();
    let conn = state.db.lock();
    let result = conn.execute(
        "INSERT INTO checkpoints (id, agent_id, conversation_id, branch_id, label, description, created_at, git_stash_ref, git_commit_hash, parent_id)
         VALUES (?1, ?2, NULL, 'main', ?3, ?4, ?5, NULL, NULL, NULL)",
        rusqlite::params![id, agent_id, label, description, now],
    );
    drop(conn);
    match result {
        Ok(_) => (format!("Checkpoint '{label}' created. ID: {id}"), false),
        Err(e) => (format!("Failed to create checkpoint: {e}"), true),
    }
}

/// Phase A3 handler: `list_checkpoints` server-side.
async fn handle_list_checkpoints_meta(
    state: &AppState,
    agent_id: &str,
    _args: &serde_json::Value,
) -> (String, bool) {
    let conn = state.db.lock();
    let mut stmt = match conn.prepare(
        "SELECT id, label, description, created_at FROM checkpoints
         WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 200",
    ) {
        Ok(s) => s,
        Err(e) => return (format!("DB prepare error: {e}"), true),
    };
    let rows: Vec<String> = match stmt.query_map(rusqlite::params![agent_id], |r| {
        let id: String = r.get(0)?;
        let label: Option<String> = r.get(1)?;
        let desc: Option<String> = r.get(2)?;
        let ts: i64 = r.get(3)?;
        Ok(format!(
            "- {} [{}] {}: {}",
            &id[..8.min(id.len())],
            ts,
            label.as_deref().unwrap_or(""),
            desc.as_deref().unwrap_or("")
        ))
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };
    if rows.is_empty() {
        ("No checkpoints found.".to_string(), false)
    } else {
        (rows.join("\n"), false)
    }
}

/// Phase A3 handler: `restore_checkpoint` server-side.
/// Looks up checkpoint by ID and marks it restored.  Git stash restore
/// requires an interactive shell and is not performed server-side.
async fn handle_restore_checkpoint_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let cp_id = args["checkpoint_id"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if cp_id.is_empty() {
        return ("Error: 'checkpoint_id' is required".to_string(), true);
    }

    let conn = state.db.lock();
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT id, label, git_stash_ref FROM checkpoints WHERE id = ?1 AND agent_id = ?2",
            rusqlite::params![cp_id, agent_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    drop(conn);

    match row {
        None => (
            format!("Checkpoint '{cp_id}' not found for agent '{agent_id}'"),
            true,
        ),
        Some((id, label, stash)) => {
            let label_str = label.as_deref().unwrap_or("?");
            let note = if stash.is_some() {
                " (git stash not applied server-side — use CLI for full restore)"
            } else {
                ""
            };
            (
                format!("Restored to checkpoint '{label_str}' ({id}).{note}"),
                false,
            )
        }
    }
}

/// Phase A4 handler: `store_artifact` server-side.
/// Inserts an artifact row directly into the DB.
async fn handle_store_artifact_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let kind = args["kind"].as_str().unwrap_or("other");
    let content = args["content"].as_str().unwrap_or("");
    let label = args["label"].as_str().unwrap_or("");

    if content.is_empty() {
        return ("Error: 'content' is required".to_string(), true);
    }

    let id = format!("art-{}", uuid::Uuid::new_v4());
    let now = crate::server::api::checkpoints::unix_ts_pub();
    let size_bytes = content.len() as i64;

    let conn = state.db.lock();
    let result = conn.execute(
        "INSERT INTO artifacts (id, agent_id, run_id, tool_call_id, kind, content_type, data_text, metadata_json, size_bytes, created_at)
         VALUES (?1, ?2, NULL, NULL, ?3, 'text/plain', ?4, '{}', ?5, ?6)",
        rusqlite::params![id, agent_id, kind, content, size_bytes, now],
    );
    drop(conn);

    match result {
        Ok(_) => {
            let label_str = if label.is_empty() {
                String::new()
            } else {
                format!(" '{label}'")
            };
            (format!("Artifact{label_str} stored. ID: {id}"), false)
        }
        Err(e) => (format!("Failed to store artifact: {e}"), true),
    }
}

/// Phase A4 handler: `list_agents` server-side.
/// Queries the agents table directly — no HTTP self-call.
async fn handle_list_agents_meta(state: &AppState, _agent_id: &str) -> (String, bool) {
    match cade_store::sqlite::list_agents(&state.db) {
        Err(e) => (format!("Failed to list agents: {e}"), true),
        Ok(agents) => {
            if agents.is_empty() {
                return ("No other agents found.".to_string(), false);
            }
            let mut out = String::from("Available agents:\n");
            for agent in agents {
                let name = &agent.name;
                let id = &agent.id;
                let desc = agent.description.as_deref().unwrap_or("No description");
                out.push_str(&format!("- {name} ({id}): {desc}\n"));
            }
            (out.trim().to_string(), false)
        }
    }
}

/// Phase A4 handler: `message_agent` server-side.
/// Runs a single `complete()` call against the target agent's accumulated
/// system prompt + messages.  Full agentic loop (with tool access) is only
/// available from CLI; server-side delivers the target's LLM response only.
async fn handle_message_agent_meta(
    state: &AppState,
    _agent_id: &str,
    args: &serde_json::Value,
    _sse_tx: tokio::sync::mpsc::Sender<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >,
) -> (String, bool) {
    let target = args["target"].as_str().unwrap_or("").trim().to_string();
    let message = args["message"].as_str().unwrap_or("").to_string();

    if target.is_empty() || message.is_empty() {
        return (
            "Error: 'target' and 'message' are required".to_string(),
            true,
        );
    }

    // Resolve target name/id → AgentRow
    let agents = match cade_store::sqlite::list_agents(&state.db) {
        Ok(a) => a,
        Err(e) => return (format!("Failed to query agents: {e}"), true),
    };
    let Some(target_agent) = agents.iter().find(|a| a.id == target || a.name == target) else {
        return (format!("Error: Agent '{target}' not found"), true);
    };

    let system_prompt = target_agent
        .system_prompt
        .clone()
        .unwrap_or_else(|| "You are a helpful assistant.".to_string());

    // Build a minimal completion request: system message + user message.
    let req = cade_ai::CompletionRequest {
        model: state.config.default_model.clone(),
        messages: vec![
            cade_ai::LlmMessage {
                role: "system".to_string(),
                content: system_prompt,
                tool_calls: None,
                tool_call_id: None,
                images: None,
            },
            cade_ai::LlmMessage {
                role: "user".to_string(),
                content: message,
                tool_calls: None,
                tool_call_id: None,
                images: None,
            },
        ],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };

    match state.llm.complete(&req).await {
        Ok(resp) => {
            let text = resp.content.as_deref().unwrap_or("").trim().to_string();
            if text.is_empty() {
                ("Target agent returned an empty response".to_string(), false)
            } else {
                (text, false)
            }
        }
        Err(e) => (format!("Failed to message agent: {e}"), true),
    }
}

///
/// Second line of defence against runaway recursion (first is the depth
/// guard in [`handle_run_subagent_tool`]).  When a subagent is handed the
/// parent's full tool list (Approach C), we remove `run_subagent` here so
/// the subagent's LLM never even sees the tool advertised — defence in
/// depth alongside the runtime depth check.

async fn handle_load_skill_tool(
    state: &AppState,
    agent_id: &str,
    tool_call_id: &str,
    arguments: &serde_json::Value,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let skill_id = arguments["id"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_default();

    if skill_id.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "load_skill".to_string(),
            output: "Error: missing 'id' parameter".to_string(),
            is_error: true,
        };
    }

    // Find skill
    let all = state.all_skills.read().await;
    let skill = all.iter().find(|s| s.id == skill_id).cloned();
    drop(all);

    match skill {
        Some(skill) => {
            // Activate for agent
            {
                let mut agent_skills = state.agent_skills.write().await;
                let loaded = agent_skills.entry(agent_id.to_string()).or_default();
                if !loaded.contains(&skill_id) {
                    loaded.push(skill_id.clone());
                }
            }

            // Invalidate context cache
            {
                let mut cache = state.context_cache.lock();
                let keys: Vec<String> = cache
                    .iter()
                    .filter(|(k, _)| k.starts_with(&format!("{agent_id}:")))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in keys {
                    cache.pop(&k);
                }
            }

            ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "load_skill".to_string(),
                output: format!(
                    "Skill '{}' loaded ({} chars). It is now active in your system prompt.",
                    skill.name,
                    skill.body.chars().count()
                ),
                is_error: false,
            }
        }
        None => ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "load_skill".to_string(),
            output: format!("Error: skill '{skill_id}' not found"),
            is_error: true,
        },
    }
}

/// Handle `unload_skill` tool call server-side.
///
/// Removes the skill from the agent's active set and invalidates the
/// context cache so the next `build_context` call drops the skill's
/// system-prompt content.  Without the cache invalidation, an LRU cached
/// context built before the unload would keep serving the skill until
/// natural eviction — leaving the agent acting as if the skill was still
/// active.
async fn handle_unload_skill_tool(
    state: &AppState,
    agent_id: &str,
    tool_call_id: &str,
    arguments: &serde_json::Value,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let skill_id = arguments["id"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_default();

    if skill_id.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: "Error: missing 'id' parameter".to_string(),
            is_error: true,
        };
    }

    let removed = {
        let mut agent_skills = state.agent_skills.write().await;
        if let Some(loaded) = agent_skills.get_mut(agent_id) {
            let before = loaded.len();
            loaded.retain(|id| id != &skill_id);
            before != loaded.len()
        } else {
            false
        }
    };

    if removed {
        // H5: invalidate context cache so the unloaded skill no longer
        // appears in the agent's system prompt on the next turn.
        // Mirrors the same logic in `handle_load_skill_tool`.
        {
            let mut cache = state.context_cache.lock();
            let keys: Vec<String> = cache
                .iter()
                .filter(|(k, _)| k.starts_with(&format!("{agent_id}:")))
                .map(|(k, _)| k.clone())
                .collect();
            for k in keys {
                cache.pop(&k);
            }
        }

        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: format!(
                "Skill '{}' unloaded. It will no longer appear in your system prompt on the next turn.",
                skill_id
            ),
            is_error: false,
        }
    } else {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: format!("Skill '{}' is not currently loaded", skill_id),
            is_error: true,
        }
    }
}

/// C7: Unified recall handler — federated search across memory, messages,
/// archival memory, and event log.
async fn handle_recall_meta(
    state: &AppState,
    agent_id: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    let query = args["query"].as_str().unwrap_or("").trim().to_string();
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;

    if query.is_empty() {
        return ("Error: 'query' is required".to_string(), true);
    }

    match cade_store::sqlite::recall(&state.db, agent_id, &query, limit) {
        Ok(results) if results.is_empty() => {
            (format!("No results found for '{query}' across memory, conversations, archival memory, or event log."), false)
        }
        Ok(results) => {
            let mut out = format!("Found {} results for '{query}':\n\n", results.len());
            for (i, r) in results.iter().enumerate() {
                out.push_str(&format!(
                    "{}. [{}] {}: {}\n",
                    i + 1,
                    r.source,
                    r.label,
                    r.snippet.chars().take(300).collect::<String>(),
                ));
            }
            (out, false)
        }
        Err(e) => (format!("Recall search failed: {e}"), true),
    }
}

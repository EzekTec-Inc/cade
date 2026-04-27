//! Async task methods and command dispatch for [`super::CadeApp`].
//!
//! All `spawn_*` helpers and `dispatch_palette_cmd` live here so that
//! `mod.rs` stays focused on rendering and action-handling.  Every
//! method clones what it needs (session `Rc`, `ctx`, URLs, tokens)
//! before entering `spawn_local` to avoid lifetime issues.

#![allow(clippy::too_many_lines)]

use std::rc::Rc;

use crate::login::LoginState;
use crate::session::SessionState;

use super::CadeApp;

impl CadeApp {
    pub(super) fn spawn_connect(&mut self, token: &str) {
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        let server_url = self.server_url.clone();
        let token = token.to_string();

        // Transition to Connecting immediately.
        *session.borrow_mut() = Some(SessionState::start(&server_url, &token));
        self.connect_started = true;
        ctx.request_repaint();

        wasm_bindgen_futures::spawn_local(async move {
            // Step 1: health check
            match crate::http_wasm::get_health(&server_url, &token).await {
                Ok(health) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_health(health);
                    }
                    ctx.request_repaint();
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_error(e.to_string());
                    }
                    ctx.request_repaint();
                    return;
                }
            }

            // Step 2: agent list
            match crate::http_wasm::get_agents(&server_url, &token).await {
                Ok(agents) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_agents(agents);
                    }
                    // Persist credentials so the next page load skips login.
                    crate::storage::save(crate::storage::StorageKey::ApiToken, &token);
                    crate::storage::save(crate::storage::StorageKey::ServerUrl, &server_url);
                    ctx.request_repaint();
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_error(e.to_string());
                    }
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Reset to login screen (called when the "Retry" button is clicked).
    pub(super) fn retry(&mut self) {
        self.login = LoginState::new();
        *self.session.borrow_mut() = None;
        self.connect_started = false;
    }

    /// Clear saved credentials and return to the login screen.
    pub(super) fn logout(&mut self) {
        crate::storage::clear_all();
        self.retry();
    }

    /// Select an agent and spawn an async task to fetch its messages.
    pub(super) fn spawn_fetch_messages(&mut self, idx: usize) {
        // Extract what we need while holding the borrow briefly.
        let (changed, server_url, token, agent_id, saved_theme) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            // Capture the selected agent's persisted theme (if any) before
            // the on_select_agent() call, so we can restore it after
            // selection — Phase 5: GUI theme persistence across reloads.
            let saved_theme = s.agents().get(idx).and_then(|a| a.theme.clone());
            let changed = s.on_select_agent(idx);
            if !changed {
                return;
            }
            let server_url = s.server_url().to_string();
            let token = s.token().to_string();
            let agent_id = s.selected_agent_id().unwrap().to_string();
            (changed, server_url, token, agent_id, saved_theme)
        };

        if !changed {
            return;
        }

        // Restore persisted theme if the agent has one.  Fires before the
        // messages fetch; theme_update SSE comes back asynchronously.
        if let Some(name) = saved_theme {
            self.spawn_apply_theme(name);
        }

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_messages_paged(&server_url, &token, &agent_id, 50, 0, None)
                .await
            {
                Ok((msgs, has_more)) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_messages_paged(msgs, has_more);
                    }
                }
                Err(_e) => {
                    // Silently ignore message-fetch errors for now —
                    // the timeline just stays empty.
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch conversations for the selected agent.
    pub(super) fn spawn_fetch_conversations(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_conversations(&server_url, &token, &agent_id).await {
                Ok(convs) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_conversations(convs);
                    }
                }
                Err(_e) => {
                    // Silently ignore — conversations list stays empty.
                }
            }
            ctx.request_repaint();
        });
    }

    /// `DELETE /v1/agents/:id/conversations/:conv_id` — delete a conversation
    /// by its list index.  On success, removes it locally via
    /// `on_conversation_deleted`.  Pushes an error toast on failure.
    pub(super) fn spawn_delete_conversation(&mut self, idx: usize) {
        let (server_url, token, agent_id, conv_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let conv_id = match s.conversations().get(idx) {
                Some(c) => c.id.clone(),
                None => return,
            };
            (
                s.server_url().to_string(),
                s.token().to_string(),
                agent_id,
                conv_id,
            )
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::delete_conversation(&server_url, &token, &agent_id, &conv_id)
                .await
            {
                Ok(()) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_conversation_deleted(idx);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.push_error(&format!("Delete conversation failed: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch messages for the currently selected conversation.
    pub(super) fn spawn_fetch_conversation_messages(&mut self) {
        let (server_url, token, agent_id, conv_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let conv_id = match s.conversation_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (
                s.server_url().to_string(),
                s.token().to_string(),
                agent_id,
                conv_id,
            )
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_messages_paged(
                &server_url,
                &token,
                &agent_id,
                50,
                0,
                Some(&conv_id),
            )
            .await
            {
                Ok((msgs, has_more)) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_messages_paged(msgs, has_more);
                    }
                }
                Err(_e) => {}
            }
            ctx.request_repaint();
        });
    }

    /// Load older messages (pagination) — fetch with offset and prepend.
    pub(super) fn spawn_load_more_messages(&mut self) {
        const PAGE_SIZE: usize = 50;
        let (server_url, token, agent_id, offset, conv_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (
                s.server_url().to_string(),
                s.token().to_string(),
                agent_id,
                s.message_count(),
                s.conversation_id().map(String::from),
            )
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_messages_paged(
                &server_url,
                &token,
                &agent_id,
                PAGE_SIZE,
                offset,
                conv_id.as_deref(),
            )
            .await
            {
                Ok((msgs, has_more)) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_prepend_messages(msgs, has_more);
                    }
                }
                Err(_e) => {}
            }
            ctx.request_repaint();
        });
    }

    /// Fetch memory blocks for the selected agent.  Assumes the overlay
    /// has already been marked as loading by the caller.
    pub(super) fn spawn_fetch_memory(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_memory(&server_url, &token, &agent_id).await {
                Ok(blocks) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_memory_loaded(blocks);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_memory_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Save the currently-edited memory block via `PUT /v1/agents/:id/memory/:label`.
    pub(super) fn spawn_save_memory_block(&mut self) {
        let (server_url, token, agent_id, label, value) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let (label, value) = match s.memory_selected_label_value() {
                Some(pair) => pair,
                None => return,
            };
            s.on_memory_save_start();
            (
                s.server_url().to_string(),
                s.token().to_string(),
                agent_id,
                label,
                value,
            )
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::put_memory_block(
                &server_url,
                &token,
                &agent_id,
                &label,
                &value,
                None,
            )
            .await
            {
                Ok(()) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_memory_save_ok();
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_memory_error(&format!("Save failed: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Update the current agent's model via `PATCH /v1/agents/:id`.  On
    /// success refreshes the agents list so the sidebar reflects the change.
    /// Fetch available models and populate the model picker overlay.
    /// The overlay must already be open (so `model_picker_loading` is set).
    pub(super) fn spawn_fetch_models(&mut self) {
        let (server_url, token) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string())
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_models(&server_url, &token).await {
                Ok((models, custom_providers)) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_models_loaded(models, custom_providers);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_models_error(format!("Failed to load models: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    pub(super) fn spawn_set_agent_model(&mut self, model: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::patch_agent_model(&server_url, &token, &agent_id, &model).await
            {
                Ok(()) => {
                    // Refetch the agents list so the sidebar shows the new model.
                    if let Ok(agents) = crate::http_wasm::get_agents(&server_url, &token).await {
                        if let Some(s) = session.borrow_mut().as_mut() {
                            s.refresh_agents(agents);
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.push_error(&format!("Model update failed: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Update the agent's compaction (summarisation) model and refresh the
    /// agents list so the new value is visible in the sidebar.  Empty
    /// `model` clears the override on the server.
    pub(super) fn spawn_set_agent_compaction_model(&mut self, model: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::patch_agent_compaction_model(
                &server_url,
                &token,
                &agent_id,
                &model,
            )
            .await
            {
                Ok(()) => {
                    if let Ok(agents) = crate::http_wasm::get_agents(&server_url, &token).await
                        && let Some(s) = session.borrow_mut().as_mut()
                    {
                        s.refresh_agents(agents);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.push_error(&format!("Compaction model update failed: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    // ── Checkpoints spawn helpers (M17) ─────────────────────────────

    /// Fetch checkpoints for the selected agent.  Assumes the overlay
    /// has already been opened (so loading flag is set).
    pub(super) fn spawn_fetch_checkpoints(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_checkpoints(&server_url, &token, &agent_id).await {
                Ok(rows) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_loaded(rows);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Restore a checkpoint.  Refreshes the list on success so the
    /// user sees any new auto-save entries the server may have added.
    pub(super) fn spawn_restore_checkpoint(&mut self, cp_id: String) {
        let (server_url, token, agent_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            s.on_checkpoints_action_start();
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result =
                crate::http_wasm::restore_checkpoint(&server_url, &token, &agent_id, &cp_id).await;
            match result {
                Ok(()) => {
                    // Truncate the id for the notice so the banner stays
                    // compact.  We only need a cue, not the full UUID.
                    let short = cp_id.chars().take(12).collect::<String>();
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_action_ok(&format!("Restored {short}…"));
                    }
                    // Refresh to pick up any new auto-save entries.
                    if let Ok(rows) =
                        crate::http_wasm::get_checkpoints(&server_url, &token, &agent_id).await
                    {
                        if let Some(s) = session.borrow_mut().as_mut() {
                            s.on_checkpoints_loaded(rows);
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Delete a checkpoint.  Refreshes the list on success.
    pub(super) fn spawn_delete_checkpoint(&mut self, cp_id: String) {
        let (server_url, token, agent_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            s.on_checkpoints_action_start();
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result =
                crate::http_wasm::delete_checkpoint(&server_url, &token, &agent_id, &cp_id).await;
            match result {
                Ok(()) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_action_ok("Deleted checkpoint");
                    }
                    if let Ok(rows) =
                        crate::http_wasm::get_checkpoints(&server_url, &token, &agent_id).await
                    {
                        if let Some(s) = session.borrow_mut().as_mut() {
                            s.on_checkpoints_loaded(rows);
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_checkpoints_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    // ── Artifacts spawn helpers (M17) ───────────────────────────────

    /// Fetch the artifact list for the selected agent.
    pub(super) fn spawn_fetch_artifacts(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_artifacts(&server_url, &token, &agent_id).await {
                Ok(rows) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_artifacts_loaded(rows);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_artifacts_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch full detail for a specific artifact (invoked after
    /// `select_artifact` has already flipped busy + cleared stale detail).
    pub(super) fn spawn_fetch_artifact_detail(&mut self, art_id: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_artifact(&server_url, &token, &agent_id, &art_id).await {
                Ok(d) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_artifact_detail_loaded(d);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_artifacts_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Delete an artifact and refresh the list.
    pub(super) fn spawn_delete_artifact(&mut self, art_id: String) {
        let (server_url, token, agent_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            s.on_artifacts_action_start();
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result =
                crate::http_wasm::delete_artifact(&server_url, &token, &agent_id, &art_id).await;
            match result {
                Ok(()) => {
                    if let Ok(rows) =
                        crate::http_wasm::get_artifacts(&server_url, &token, &agent_id).await
                    {
                        if let Some(s) = session.borrow_mut().as_mut() {
                            s.on_artifacts_loaded(rows);
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_artifacts_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    // ── Metrics + context spawn helpers (M19) ──────────────────────

    /// Phase-3 `/compact` slash command: synchronously trigger
    /// session_summary consolidation and surface the result via toast.
    pub(super) fn spawn_compact(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        // Pre-toast so the user sees immediate feedback while the
        // request is in flight.
        if let Some(s) = self.session.borrow_mut().as_mut() {
            s.push_info("Compacting context — consolidating dropped turns…");
        }

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::http_wasm::compact(&server_url, &token, &agent_id).await;
            if let Some(s) = session.borrow_mut().as_mut() {
                match result {
                    Ok(chars) if chars > 0 => {
                        s.push_info(&format!(
                            "✓ Context compacted (session_summary: {chars} chars)"
                        ));
                    }
                    Ok(_) => {
                        s.push_info("✓ Compact triggered (nothing to consolidate yet)");
                    }
                    Err(e) => {
                        s.push_error(&format!("Compact failed: {e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    pub(super) fn spawn_fetch_metrics(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(m) = crate::http_wasm::get_metrics(&server_url, &token, &agent_id).await {
                if let Some(s) = session.borrow_mut().as_mut() {
                    s.on_metrics_loaded(m);
                }
                ctx.request_repaint();
            }
        });
    }

    pub(super) fn spawn_fetch_context_stats(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_context_stats(&server_url, &token, &agent_id).await {
                Ok(stats) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_context_loaded(stats);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_context_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch per-category context breakdown from the server.
    pub(super) fn spawn_fetch_context_breakdown(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_context_breakdown(&server_url, &token, &agent_id).await {
                Ok(breakdown) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_context_breakdown(breakdown);
                    }
                }
                Err(_) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_context_breakdown_error();
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    // ── Tools spawn helper (M18) ────────────────────────────────────

    pub(super) fn spawn_fetch_tools(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_tools(&server_url, &token, &agent_id).await {
                Ok(tools) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_tools_loaded(tools);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_tools_error(&format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch the server-wide MCP server list (`GET /v1/mcp`) and populate
    /// the MCP overlay.  The overlay must already be open so the loading
    /// flag is set before this runs.
    pub(super) fn spawn_fetch_mcp(&mut self) {
        let (server_url, token) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string())
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_mcp_status(&server_url, &token).await {
                Ok(servers) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_mcp_loaded(servers);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_mcp_error(format!("{e}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// Call `on_send` on the session state, then spawn an async SSE stream
    /// that feeds chunks back into the session.
    pub(super) fn spawn_stream_message(&mut self) {
        // on_send returns the trimmed input if the send is valid.
        let (input, server_url, token, agent_id, conv_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let input = match s.on_send() {
                Some(i) => i,
                None => return,
            };
            let server_url = s.server_url().to_string();
            let token = s.token().to_string();
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let conv_id = s.conversation_id().map(String::from);
            (input, server_url, token, agent_id, conv_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let session_clone = Rc::clone(&session);
            let ctx_clone = ctx.clone();

            let result = crate::http_wasm::send_message_stream(
                &server_url,
                &token,
                &agent_id,
                &input,
                conv_id.as_deref(),
                move |evt| {
                    use crate::api::StreamEvent;
                    if let Some(s) = session_clone.borrow_mut().as_mut() {
                        match evt {
                            StreamEvent::ConversationId(cid) => s.on_conversation_id(&cid),
                            StreamEvent::Text(text) => s.on_stream_chunk(&text),
                            StreamEvent::Reasoning(text) => s.on_stream_reasoning(&text),
                            StreamEvent::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                s.on_stream_tool_call(&id, &name, &arguments);
                            }
                            StreamEvent::ToolResult {
                                id,
                                name,
                                output,
                                is_error,
                            } => {
                                s.on_stream_tool_result(&id, &name, &output, is_error);
                            }
                            StreamEvent::Usage {
                                input_tokens,
                                output_tokens,
                                model,
                            } => {
                                s.on_usage(input_tokens, output_tokens, model.as_deref());
                            }
                            StreamEvent::FinishReason(reason) => {
                                s.on_finish_reason(&reason);
                            }
                            StreamEvent::ThemeUpdate(theme) => {
                                s.on_theme_update(theme);
                            }
                            StreamEvent::SubagentStarted {
                                subagent_id,
                                task,
                                mode,
                                model,
                            } => {
                                s.on_subagent_started(&subagent_id, &task, &mode, &model);
                            }
                            StreamEvent::SubagentProgress {
                                subagent_id,
                                status,
                                tool_calls,
                                output_lines,
                                elapsed_secs,
                            } => {
                                s.on_subagent_progress(
                                    &subagent_id,
                                    &status,
                                    tool_calls,
                                    output_lines,
                                    elapsed_secs,
                                );
                            }
                            StreamEvent::SubagentComplete {
                                subagent_id,
                                status,
                                result_preview,
                                elapsed_secs,
                                is_error,
                            } => {
                                s.on_subagent_complete(
                                    &subagent_id,
                                    &status,
                                    &result_preview,
                                    elapsed_secs,
                                    is_error,
                                );
                            }
                            StreamEvent::SystemNotice {
                                level: _,
                                code: _,
                                message,
                            } => {
                                if !message.is_empty() {
                                    // Phase 3: surface the server-side
                                    // notice as a toast in the GUI.
                                    s.push_info(&message);
                                }
                            }
                        }
                    }
                    ctx_clone.request_repaint();
                },
            )
            .await;

            // Mark stream as done and surface any error.
            if let Some(s) = session.borrow_mut().as_mut() {
                match result {
                    Ok(()) => s.on_stream_done(),
                    Err(e) => s.push_error(&format!("{e}")),
                }
            }

            ctx.request_repaint();
        });
    }

    /// Execute a palette command.  Called after the palette overlay has
    /// been closed, so all session borrows are released.  Each command
    /// maps to an existing in-app behavior (logout, new conversation,
    /// clear, etc.) or surfaces a toast for not-yet-implemented entries.
    pub(super) fn dispatch_palette_cmd(&mut self, cmd: crate::palette::PaletteCmd) {
        use crate::palette::PaletteCmd;
        match cmd {
            PaletteCmd::Logout => self.logout(),
            PaletteCmd::New => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.on_new_conversation();
                }
            }
            PaletteCmd::Clear => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.clear_timeline_local();
                }
            }
            PaletteCmd::Copy => {
                let text = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.last_assistant_content());
                match text {
                    Some(t) => {
                        self.ctx.copy_text(t);
                    }
                    None => {
                        if let Some(s) = self.session.borrow_mut().as_mut() {
                            s.push_error("No assistant message to copy");
                        }
                    }
                }
            }
            PaletteCmd::Help => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_menu("");
                }
            }
            PaletteCmd::Memory => {
                // Require an agent to be selected.
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before viewing memory");
                    }
                    return;
                }
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_memory_overlay();
                }
                self.spawn_fetch_memory();
            }
            PaletteCmd::Model(model) => {
                let model = model.trim().to_string();
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before changing model");
                    }
                    return;
                }
                if model.is_empty() {
                    // No arg → open the model picker overlay and fetch models
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.open_model_picker();
                    }
                    self.spawn_fetch_models();
                } else {
                    self.spawn_set_agent_model(model);
                }
            }
            PaletteCmd::Checkpoints => {
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before viewing checkpoints");
                    }
                    return;
                }
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_checkpoints_overlay();
                }
                self.spawn_fetch_checkpoints();
            }
            PaletteCmd::Artifacts => {
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before viewing artifacts");
                    }
                    return;
                }
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_artifacts_overlay();
                }
                self.spawn_fetch_artifacts();
            }
            PaletteCmd::Unknown(raw) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.push_error(&format!("Unknown command: /{raw}"));
                }
            }
            PaletteCmd::Providers => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        providers_open,
                        providers_loading,
                        ..
                    } = s
                    {
                        *providers_open = true;
                        *providers_loading = true;
                    }
                }
                self.spawn_fetch_providers();
            }
            PaletteCmd::Permissions => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        permissions_open, ..
                    } = s
                    {
                        *permissions_open = true;
                    }
                }
            }
            PaletteCmd::Theme => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        theme_picker_open, ..
                    } = s
                    {
                        *theme_picker_open = true;
                    }
                }
            }
            PaletteCmd::Hooks => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { hooks_open, .. } = s {
                        *hooks_open = true;
                    }
                }
            }
            PaletteCmd::Mode(_mode_arg) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        permissions_open, ..
                    } = s
                    {
                        *permissions_open = true;
                    }
                }
            }
            PaletteCmd::Toolset(_ts_arg) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { toolset_open, .. } = s {
                        *toolset_open = true;
                    }
                }
            }
            PaletteCmd::Pricing => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { pricing_open, .. } = s {
                        *pricing_open = true;
                    }
                }
            }
            PaletteCmd::Backend(_be_arg) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { backend_open, .. } = s {
                        *backend_open = true;
                    }
                }
            }
            PaletteCmd::Reasoning(_level_arg) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { reasoning_open, .. } = s {
                        *reasoning_open = true;
                    }
                }
            }
            PaletteCmd::Compact => {
                self.spawn_compact();
            }
            PaletteCmd::CompactionModel(model) => {
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before changing compaction model");
                    }
                    return;
                }
                let model = model.trim().to_string();
                if model.is_empty() {
                    // Mirror CLI ergonomics: empty arg in palette is treated as
                    // a usage prompt rather than an implicit clear, because the
                    // palette has no confirmation step. Explicit clear is still
                    // available via the CLI `/compaction-model` (no arg).
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Usage: /compaction-model <model-id>  (use the CLI to clear)");
                    }
                } else {
                    self.spawn_set_agent_compaction_model(model);
                }
            }
            PaletteCmd::Unsupported(name) => {
                // TUI recognizes this command but the GUI has no UI or
                // backing action for it yet.  Surface a message that
                // tells the user exactly which command and where to
                // reach it today (the CLI / TUI).
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.push_error(&format!(
                        "/{name} is available in the CADE CLI/TUI — GUI panel coming soon"
                    ));
                }
            }
            PaletteCmd::Skills => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        skills_overlay_open,
                        skills_loading,
                        ..
                    } = s
                    {
                        *skills_overlay_open = true;
                        *skills_loading = true;
                    }
                }
                self.spawn_fetch_skills();
                self.spawn_fetch_tools();
            }
            PaletteCmd::Mcp => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_mcp_overlay();
                }
                self.spawn_fetch_mcp();
            }
            PaletteCmd::Agents => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_agents_overlay();
                }
            }
            PaletteCmd::Agent(name) => {
                // Switch to agent by matching name or id prefix (case-insensitive).
                let name_lc = name.trim().to_lowercase();
                if name_lc.is_empty() {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Usage: /agent <name-or-id>");
                    }
                    return;
                }
                let idx = self.session.borrow().as_ref().and_then(|s| {
                    let agents = s.agents();
                    agents.iter().position(|a| {
                        a.name.to_lowercase().contains(&name_lc)
                            || a.id.to_lowercase().starts_with(&name_lc)
                    })
                });
                match idx {
                    Some(i) => {
                        self.spawn_fetch_messages(i);
                        self.spawn_fetch_conversations();
                        self.spawn_fetch_metrics();
                        if let Some(s) = self.session.borrow_mut().as_mut() {
                            s.on_select_agent(i);
                        }
                    }
                    None => {
                        if let Some(s) = self.session.borrow_mut().as_mut() {
                            s.push_error(&format!("No agent matching '{name}'"));
                        }
                    }
                }
            }
            PaletteCmd::Context => {
                let has_agent = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_agent_id().map(|_| ()))
                    .is_some();
                if !has_agent {
                    if let Some(s) = self.session.borrow_mut().as_mut() {
                        s.push_error("Select an agent before viewing context stats");
                    }
                    return;
                }
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_context_overlay();
                }
                self.spawn_fetch_context_stats();
                self.spawn_fetch_context_breakdown();
            }
            PaletteCmd::Stats => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_stats_overlay();
                }
            }
            PaletteCmd::Search(_) => {
                // Client-side message search is not yet implemented.
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.push_error("/search is not yet implemented in the GUI");
                }
            }
        }
    }

    // ── Provider / Skills / Reasoning wiring ────────────────────────────────

    /// Fetch configured providers from server and store in session.
    pub(super) fn spawn_fetch_providers(&mut self) {
        let (server_url, token) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string())
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_providers(&server_url, &token).await {
                Ok(providers) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_providers_loaded(providers);
                    }
                }
                Err(e) => {
                    web_sys::console::warn_1(&format!("fetch providers: {e:?}").into());
                }
            }
            ctx.request_repaint();
        });
    }

    /// Fetch all discovered skills + agent's loaded skills.
    pub(super) fn spawn_fetch_skills(&mut self) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let all = crate::http_wasm::get_all_skills(&server_url, &token)
                .await
                .unwrap_or_default();
            let loaded = crate::http_wasm::get_agent_skills(&server_url, &token, &agent_id)
                .await
                .unwrap_or_default();
            if let Some(s) = session.borrow_mut().as_mut() {
                s.on_skills_loaded(all, loaded);
            }
            ctx.request_repaint();
        });
    }

    /// POST load a skill for the current agent.
    pub(super) fn spawn_load_skill(&mut self, skill_id: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::post_load_skill(&server_url, &token, &agent_id, &skill_id).await
            {
                Ok(()) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_skill_loaded(&skill_id);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.push_error(&format!("load skill: {e:?}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// POST unload a skill for the current agent.
    pub(super) fn spawn_unload_skill(&mut self, skill_id: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::post_unload_skill(&server_url, &token, &agent_id, &skill_id)
                .await
            {
                Ok(()) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_skill_unloaded(&skill_id);
                    }
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.push_error(&format!("unload skill: {e:?}"));
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    /// PATCH reasoning effort to server.
    pub(super) fn spawn_patch_reasoning(&mut self, effort: String) {
        let (server_url, token, agent_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            (s.server_url().to_string(), s.token().to_string(), agent_id)
        };
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ =
                crate::http_wasm::patch_agent_reasoning(&server_url, &token, &agent_id, &effort)
                    .await;
            ctx.request_repaint();
        });
    }

    /// Send `/theme <name>` silently through the run endpoint.
    ///
    /// The server intercepts the `/theme` prefix, resolves the named theme
    /// from disk, and broadcasts a `theme_update` SSE event.  The GUI's
    /// existing `on_theme_update` handler then applies the new colors.
    ///
    /// We use `send_message_stream` with a no-op event handler so the theme
    /// command doesn't pollute the chat timeline.
    pub(super) fn spawn_apply_theme(&mut self, name: String) {
        let (server_url, token, agent_id, conv_id) = {
            let session = self.session.borrow();
            let s = match session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let conv_id = s.conversation_id().map(String::from);
            (
                s.server_url().to_string(),
                s.token().to_string(),
                agent_id,
                conv_id,
            )
        };
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let session_clone = Rc::clone(&session);
            let ctx_clone = ctx.clone();
            let _ = crate::http_wasm::send_message_stream(
                &server_url,
                &token,
                &agent_id,
                &format!("/theme {name}"),
                conv_id.as_deref(),
                move |evt| {
                    // Only handle theme_update — ignore all other events so
                    // the command stays invisible in the chat timeline.
                    if let crate::api::StreamEvent::ThemeUpdate(colors) = evt {
                        if let Some(s) = session_clone.borrow_mut().as_mut() {
                            s.on_theme_update(colors);
                        }
                        ctx_clone.request_repaint();
                    }
                },
            )
            .await;
            ctx.request_repaint();
        });
    }
}

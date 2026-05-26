//! Dashboard app — thin egui view over the login + session state machines.
//!
//! This module is wasm32-only.  All real logic lives in `login.rs` and
//! `session.rs` which are fully native-testable.  The render code here
//! contains no branching logic other than matching the state-machine
//! variants to UI strings; all state transitions are delegated to the
//! pure modules.
//!
//! ## Module layout
//!
//! | Sub-module | Contents |
//! |------------|----------|
//! | `tasks`    | All `spawn_*` async helpers + `dispatch_palette_cmd` |
//! | `views`    | `render_welcome`, `render_timeline_message` |
//! | `overlays` | One file per overlay panel (palette, memory, …) |

#![allow(clippy::too_many_lines)]

pub mod components;
pub mod overlays;

mod tasks;
mod views;

use crate::theme::EguiThemeExt;
use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;

use crate::config::Config;
use crate::login::LoginState;
use crate::session::SessionState;

// Bring view helpers into scope.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePage {
    Overview,
    Chat,
    Logs,
    Memory,
    Skills,
}

/// Top-level eframe app for the cade-gui dashboard.
pub struct CadeApp {
    /// Active tab in the dashboard.
    active_page: ActivePage,
    /// Login form state — driven by user input events.
    login: LoginState,
    /// Post-login session state — driven by async HTTP results.
    /// `None` means we haven't started connecting yet.
    session: Rc<RefCell<Option<SessionState>>>,
    /// Guard: true once we've spawned the connection task for the
    /// current `Submitted` token.  Reset on retry.
    connect_started: bool,
    /// Saved egui context for repaint requests from async tasks.
    ctx: egui::Context,
    /// Server URL resolved at boot (from page origin).
    server_url: String,
    /// Shared cache for egui_commonmark — avoids re-parsing markdown
    /// on every frame.
    theme: crate::theme::ThemeColors,
    viewport: crate::responsive::Viewport,
}

impl CadeApp {
    /// Construct from the `CreationContext` handed to us by eframe.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Apply the CADE dark theme once at startup.
        crate::theme::apply_theme(&cc.egui_ctx, &crate::theme::ThemeColors::default());

        // Resolve the server URL from the page origin.  In production
        // the dashboard is served by cade-server, so origin == API host.
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_else(|| "http://localhost:8284".to_string());
        let query = web_sys::window().and_then(|w| w.location().search().ok());
        let config = Config::resolve(&origin, query.as_deref(), None);

        let mut login = LoginState::new();

        // If a token was provided via query param (?key=...), use it immediately.
        // Otherwise, fall back to the persisted token in storage.
        if !config.api_key.is_empty() {
            login.on_input(&config.api_key);
            login.on_submit();
        } else if let Some(saved_token) = crate::storage::load(crate::storage::StorageKey::ApiToken)
        {
            if !saved_token.is_empty() {
                login.on_input(&saved_token);
                login.on_submit();
            }
        }

        Self {
            active_page: ActivePage::Overview,
            login,
            session: Rc::new(RefCell::new(None)),
            connect_started: false,
            ctx: cc.egui_ctx.clone(),
            server_url: config.server_url,
            theme: crate::theme::ThemeColors::default(),
            viewport: crate::responsive::Viewport::Desktop,
        }
    }



}

impl eframe::App for CadeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Update viewport based on current window size and apply responsive styling
        self.viewport = crate::responsive::detect(ui.ctx());
        crate::responsive::apply_style(ui.ctx(), self.viewport);

        // Collect actions during rendering so we can apply them after
        // all borrows are released.  This avoids borrow-conflict issues
        // with Rc<RefCell<..>>.
        let mut action = AppAction::None;

        // ── Global keyboard shortcuts ────────────────────────────

        // Snapshot session state once so we can read it in the toolbar
        // without holding a borrow into the render closures below.
        let session_snapshot_for_toolbar = self.session.borrow().clone();

        // ── Sidebar (Left) ───────────────────────────────────────────────────────────
        if let Some(new_action) = components::sidebar::render(ui, &self.theme) {
            action = new_action;
        }

        // ── Top toolbar (Right of Sidebar) ───────────────────────────────────────────
        if let Some(new_action) = components::header::render(
            ui,
            &mut self.active_page,
            &session_snapshot_for_toolbar,
            &self.theme,
        ) {
            action = new_action;
        }

        // ── Main Content (Central Panel) ─────────────────────────────────────────────
        if let Some(SessionState::Connected(session)) = &session_snapshot_for_toolbar {
            components::overview::render(ui, session, &self.theme);
        } else {
            // Fallback for unconnected state
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Connecting...");
                    ui.spinner();
                });
            });
        }

        // Apply deferred actions outside the ui closure.
        match action {
            AppAction::None => {}
            AppAction::Connect(token) => self.spawn_connect(&token),
            AppAction::Retry => self.retry(),
            AppAction::SelectAgent(idx) => {
                self.spawn_fetch_messages(idx);
                self.spawn_fetch_conversations();
                self.spawn_fetch_metrics();
            }
            AppAction::SendMessage => self.spawn_stream_message(),
            AppAction::DismissError => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.dismiss_error();
                }
            }
            AppAction::SelectConversation(idx) => {
                let changed = {
                    let mut session = self.session.borrow_mut();
                    match session.as_mut() {
                        Some(s) => s.on_select_conversation(idx),
                        None => false,
                    }
                };
                if changed {
                    self.spawn_fetch_conversation_messages();
                }
            }
            AppAction::DeleteConversation(idx) => {
                self.spawn_delete_conversation(idx);
            }
            AppAction::NewConversation => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.on_new_conversation();
                }
            }
            AppAction::LoadMore => self.spawn_load_more_messages(),
            AppAction::Logout => self.logout(),
            AppAction::ScrollToBottom => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.enable_auto_scroll();
                }
            }
            AppAction::DisableAutoScroll => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.disable_auto_scroll();
                }
            }
            AppAction::OpenPalette(initial) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_palette(&initial);
                }
            }
            AppAction::ClosePalette => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_palette();
                }
            }
            AppAction::SetPaletteInput(q) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.set_palette_input(&q);
                }
            }
            AppAction::MovePaletteSelection(delta) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.move_palette_selection(delta);
                }
            }
            AppAction::ExecutePaletteCmd => {
                let cmd = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_palette_cmd());
                // Always close the palette after attempting to execute.
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_palette();
                }
                if let Some(cmd) = cmd {
                    self.dispatch_palette_cmd(cmd);
                }
            }
            AppAction::OpenMenu(initial) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.open_menu(&initial);
                }
            }
            AppAction::CloseMenu => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_menu();
                }
            }
            AppAction::SetMenuInput(q) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.set_menu_input(&q);
                }
            }
            AppAction::MoveMenuSelection(delta) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.move_menu_selection(delta);
                }
            }
            AppAction::ExecuteMenuCmd => {
                let cmd = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_menu_cmd());
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_menu();
                }
                if let Some(cmd) = cmd {
                    self.dispatch_palette_cmd(cmd);
                }
            }
            AppAction::CloseMemoryOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_memory_overlay();
                }
            }
            AppAction::SelectMemoryBlock(idx) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.select_memory_block(idx);
                }
            }
            AppAction::SetMemoryEditBuffer(v) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.set_memory_edit_buffer(&v);
                }
            }
            AppAction::SaveMemoryBlock => self.spawn_save_memory_block(),
            AppAction::ToggleMemoryHistory => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.toggle_memory_history();
                }
                self.spawn_fetch_memory_history();
            }
            AppAction::RestoreMemoryRevision(rev_id) => {
                self.spawn_restore_memory_revision(rev_id);
            }
            AppAction::CloseCheckpointsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_checkpoints_overlay();
                }
            }
            AppAction::RestoreCheckpoint(id) => self.spawn_restore_checkpoint(id),
            AppAction::DeleteCheckpoint(id) => self.spawn_delete_checkpoint(id),
            AppAction::CloseArtifactsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_artifacts_overlay();
                }
            }
            AppAction::SelectArtifact(idx) => {
                // Two-step: select_artifact flips the state machine +
                // returns the id so the spawn helper can do the GET.
                let maybe_id = self
                    .session
                    .borrow_mut()
                    .as_mut()
                    .and_then(|s| s.select_artifact(idx));
                if let Some(id) = maybe_id {
                    self.spawn_fetch_artifact_detail(id);
                }
            }
            AppAction::DeleteSelectedArtifact => {
                let maybe_id = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.selected_artifact_id());
                if let Some(id) = maybe_id {
                    self.spawn_delete_artifact(id);
                }
            }
            AppAction::CloseToolsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_tools_overlay();
                }
            }
            AppAction::AnswerQuestion => {
                // Build the answer string then send it as a user message.
                let answer = self.session.borrow_mut().as_mut().and_then(|s| {
                    let a = s.commit_question_answer();
                    s.clear_active_question();
                    a
                });
                if let Some(text) = answer {
                    // Inject as next user message via the existing send path.
                    if let Some(SessionState::Connected(session)) =
                        self.session.borrow_mut().as_mut()
                    {
                        session.input_buffer = text;
                    }
                    self.spawn_stream_message();
                }
            }
            AppAction::DismissQuestion => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.clear_active_question();
                }
            }
            AppAction::MoveQuestionCursor(delta) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.move_question_cursor(delta);
                }
            }
            AppAction::ToggleQuestionChecked => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.toggle_question_checked();
                }
            }
            AppAction::CloseAgentsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_agents_overlay();
                }
            }
            AppAction::CloseContextOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_context_overlay();
                }
            }
            AppAction::CloseStatsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_stats_overlay();
                }
            }
            AppAction::CloseMcpOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_mcp_overlay();
                }
            }
            AppAction::CloseModelPicker => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_model_picker();
                }
            }
            AppAction::SetModelPickerQuery(q) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.set_model_picker_query(q);
                }
            }
            AppAction::SetModelPickerSelection(idx) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.set_model_picker_selection(idx);
                }
            }
            AppAction::SelectModel(model_id) => {
                // Close picker, then apply the model
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.close_model_picker();
                }
                self.spawn_set_agent_model(model_id);
            }

            // ── Settings overlay actions ─────────────────────────
            AppAction::ExecutePaletteCommand(trigger) => {
                let cmd = cade_core::resources::palette::parse_palette_input(&trigger);
                self.dispatch_palette_cmd(cmd);
            }
            AppAction::CloseProvidersOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let providers_open = &mut session.providers_open;
                        *providers_open = false;
                    }
                }
            }
            AppAction::ClosePermissionsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let permissions_open = &mut session.permissions_open;
                        *permissions_open = false;
                    }
                }
            }
            AppAction::SetPermissionMode(mode) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let current_permission_mode = &mut session.current_permission_mode;
                        let permissions_open = &mut session.permissions_open;
                        *current_permission_mode = mode;
                        *permissions_open = false;
                    }
                }
            }
            AppAction::CloseThemeOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let theme_picker_open = &mut session.theme_picker_open;
                        *theme_picker_open = false;
                    }
                }
            }
            AppAction::SetTheme(name) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let current_theme_name = &mut session.current_theme_name;
                        let theme_picker_open = &mut session.theme_picker_open;
                        *current_theme_name = name.clone();
                        *theme_picker_open = false;
                    }
                }
                // Send `/theme <name>` silently through the run endpoint.
                // The server intercepts it, resolves the theme from disk,
                // and broadcasts a theme_update SSE event.
                self.spawn_apply_theme(name);
            }
            AppAction::CloseHooksOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let hooks_open = &mut session.hooks_open;
                        *hooks_open = false;
                    }
                }
            }
            AppAction::CloseToolsetOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let toolset_open = &mut session.toolset_open;
                        *toolset_open = false;
                    }
                }
            }
            AppAction::SetToolset(ts) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let current_toolset = &mut session.current_toolset;
                        let toolset_open = &mut session.toolset_open;
                        *current_toolset = ts;
                        *toolset_open = false;
                    }
                }
            }
            AppAction::ClosePricingOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let pricing_open = &mut session.pricing_open;
                        *pricing_open = false;
                    }
                }
            }
            AppAction::CloseBackendOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let backend_open = &mut session.backend_open;
                        *backend_open = false;
                    }
                }
            }
            AppAction::SetBackend(be) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let current_backend = &mut session.current_backend;
                        let backend_open = &mut session.backend_open;
                        *current_backend = be;
                        *backend_open = false;
                    }
                }
            }
            AppAction::CloseReasoningOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let reasoning_open = &mut session.reasoning_open;
                        *reasoning_open = false;
                    }
                }
            }
            AppAction::SetReasoning(level) => {
                let effort_str = level.clone();
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let current_reasoning_effort = &mut session.current_reasoning_effort;
                        let reasoning_open = &mut session.reasoning_open;
                        *current_reasoning_effort = level;
                        *reasoning_open = false;
                    }
                }
                self.spawn_patch_reasoning(effort_str);
            }
            AppAction::CloseSkillsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let skills_overlay_open = &mut session.skills_overlay_open;
                        let skills_filter = &mut session.skills_filter;
                        *skills_overlay_open = false;
                        *skills_filter = String::new();
                    }
                }
            }
            AppAction::SetSkillsFilter(q) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        let skills_filter = &mut session.skills_filter;
                        *skills_filter = q;
                    }
                }
            }
            AppAction::LoadSkill(id) => {
                self.spawn_load_skill(id);
            }
            AppAction::UnloadSkill(id) => {
                self.spawn_unload_skill(id);
            }
            AppAction::ToggleProfilesOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        session.profiles_open = !session.profiles_open;
                        if session.profiles_open {
                            if let Some(json) =
                                crate::storage::load(crate::storage::StorageKey::Profiles)
                            {
                                if let Ok(p) =
                                    serde_json::from_str::<Vec<(String, String, String)>>(&json)
                                {
                                    session.profiles = p;
                                }
                            }
                        }
                    }
                }
            }
            AppAction::ConnectProfile(url, token) => {
                self.server_url = url.clone();
                self.login.on_input(&token);
                self.login.on_submit();
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        session.profiles_open = false;
                    }
                }
                self.retry(); // Disconnect current session
                self.spawn_connect(&token);
            }
            AppAction::DeleteProfile(idx) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        if idx < session.profiles.len() {
                            session.profiles.remove(idx);
                            crate::storage::save(
                                crate::storage::StorageKey::Profiles,
                                &serde_json::to_string(&session.profiles).unwrap(),
                            );
                        }
                    }
                }
            }
            AppAction::SaveProfile(name, url, token) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        session.profiles.push((name, url, token));
                        crate::storage::save(
                            crate::storage::StorageKey::Profiles,
                            &serde_json::to_string(&session.profiles).unwrap(),
                        );
                        session.profile_edit_name.clear();
                        session.profile_edit_url.clear();
                        session.profile_edit_token.clear();
                    }
                }
            }
            AppAction::SetProfileEdit(name, url, token) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected(session) = s {
                        session.profile_edit_name = name;
                        session.profile_edit_url = url;
                        session.profile_edit_token = token;
                    }
                }
            }
        }
    }
}

/// Deferred actions collected during the render pass and applied
/// after all borrows are released.
pub enum AppAction {
    None,
    Connect(String),
    Retry,
    /// User clicked an agent in the sidebar — spawn a message fetch.
    SelectAgent(usize),
    /// User submitted a message — spawn the SSE stream.
    SendMessage,
    /// User dismissed the error toast.
    DismissError,
    /// User selected a conversation in the sidebar.
    SelectConversation(usize),
    /// User clicked the delete (🗑) button on a conversation in the sidebar.
    DeleteConversation(usize),
    /// User clicked "New Chat" — start a fresh conversation.
    NewConversation,
    /// User clicked "Load more" to fetch older messages.
    LoadMore,
    /// User clicked Logout — clear credentials and return to login.
    Logout,
    /// User clicked the ↓ scroll-to-bottom button — re-enable auto-scroll.
    ScrollToBottom,
    /// Emitted when egui detects the user has scrolled up — disable auto-scroll.
    DisableAutoScroll,
    /// Open the slash-command palette.  Optional pre-filled query.
    OpenPalette(String),
    /// Close the palette without executing.
    ClosePalette,
    /// Replace palette filter query.
    SetPaletteInput(String),
    /// Move palette selection (negative = up, positive = down).
    MovePaletteSelection(i32),
    /// Execute whatever command the palette currently highlights.
    ExecutePaletteCmd,
    /// Open the full-screen command menu.
    OpenMenu(String),
    /// Close the command menu.
    CloseMenu,
    /// Replace menu filter query.
    SetMenuInput(String),
    /// Move menu selection (negative = up, positive = down).
    MoveMenuSelection(i32),
    /// Execute whatever command the menu currently highlights.
    ExecuteMenuCmd,
    /// Close the memory overlay.
    CloseMemoryOverlay,
    /// Select a memory block in the overlay sidebar.
    SelectMemoryBlock(usize),
    /// Replace the in-flight memory edit buffer (live TextEdit sync).
    SetMemoryEditBuffer(String),
    /// Save the currently-edited memory block to the server.
    SaveMemoryBlock,
    /// Toggle memory history sidebar.
    ToggleMemoryHistory,
    /// Restore a specific memory revision by id.
    RestoreMemoryRevision(String),
    /// Close the checkpoints overlay.
    CloseCheckpointsOverlay,
    /// Restore a specific checkpoint by id.
    RestoreCheckpoint(String),
    /// Delete a specific checkpoint by id.
    DeleteCheckpoint(String),
    /// Close the artifacts overlay.
    CloseArtifactsOverlay,
    /// User clicked an artifact in the list — fetch full detail.
    SelectArtifact(usize),
    /// Delete the currently-selected artifact.
    DeleteSelectedArtifact,
    /// Close the tools/MCP overlay.
    CloseToolsOverlay,
    /// User submitted an answer to the active question widget.
    AnswerQuestion,
    /// User dismissed the question widget without answering.
    DismissQuestion,
    /// Move the question cursor up (-1) or down (+1).
    MoveQuestionCursor(i32),
    /// Toggle the checked state at cursor (multi-select).
    ToggleQuestionChecked,
    /// Close the agents overlay.
    CloseAgentsOverlay,
    /// Close the context-stats overlay.
    CloseContextOverlay,
    /// Close the stats overlay.
    CloseStatsOverlay,
    /// Close the MCP servers overlay.
    CloseMcpOverlay,
    /// Close the model picker overlay.
    CloseModelPicker,
    /// Update model picker search query.
    SetModelPickerQuery(String),
    /// Move model picker selection to index.
    SetModelPickerSelection(usize),
    /// User selected a model from the picker — apply it.
    SelectModel(String),

    // ── Settings overlay actions ─────────────────────────────────
    /// Execute a palette command by trigger name (for click dispatch).
    ExecutePaletteCommand(String),
    /// Close providers overlay.
    CloseProvidersOverlay,
    /// Close permissions overlay.
    ClosePermissionsOverlay,
    /// Set permission mode.
    SetPermissionMode(String),
    /// Close theme overlay.
    CloseThemeOverlay,
    /// Set theme.
    SetTheme(String),
    /// Close hooks overlay.
    CloseHooksOverlay,
    /// Close toolset overlay.
    CloseToolsetOverlay,
    /// Set toolset.
    SetToolset(String),
    /// Close pricing overlay.
    ClosePricingOverlay,
    /// Close backend overlay.
    CloseBackendOverlay,
    /// Set backend.
    SetBackend(String),
    /// Close reasoning overlay.
    CloseReasoningOverlay,
    /// Set reasoning effort.
    SetReasoning(String),
    // ── Skills overlay actions ───────────────────────────────
    /// Close skills overlay.
    CloseSkillsOverlay,
    /// Set skills filter text.
    SetSkillsFilter(String),
    /// Load a skill by ID.
    LoadSkill(String),
    /// Unload a skill by ID.
    UnloadSkill(String),
    // ── Profiles overlay actions ───────────────────────────────
    /// Toggle profiles overlay.
    ToggleProfilesOverlay,
    /// Connect to a specific profile.
    ConnectProfile(String, String),
    /// Delete a profile.
    DeleteProfile(usize),
    /// Save a profile.
    SaveProfile(String, String, String),
    /// Update profile edit buffer.
    SetProfileEdit(String, String, String),
}

// ── M1: toolbar helpers ───────────────────────────────────────────────────────

/// Returns the colour for the live-status dot in the top toolbar.
/// `true` (streaming) → WARNING amber; `false` (idle) → SUCCESS green.
pub(crate) fn status_dot_color(
    streaming: bool,
    theme: &crate::theme::ThemeColors,
) -> egui::Color32 {
    if streaming {
        theme.warning()
    } else {
        theme.success()
    }
}

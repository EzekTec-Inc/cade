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
use egui_commonmark::CommonMarkCache;

use crate::config::Config;
use crate::login::LoginState;
use crate::session::SessionState;
use crate::shortcuts::{ShortcutAction, poll_shortcut};

// Bring overlay render functions into scope so `ui()` can call them unqualified.
use overlays::{
    render_agents_overlay, render_artifacts_overlay, render_checkpoints_overlay,
    render_context_overlay, render_mcp_overlay, render_memory_overlay, render_menu_overlay,
    render_model_picker, render_palette_overlay, render_question_widget, render_stats_overlay,
    render_tools_overlay,
};
// Bring view helpers into scope.

/// Top-level eframe app for the cade-gui dashboard.
pub struct CadeApp {
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
    md_cache: CommonMarkCache,
    /// Stable ID for the chat input field — used by Ctrl+L to request focus.
    input_id: egui::Id,
    theme: crate::theme::ThemeColors,
    viewport: crate::responsive::Viewport,
    sidebar_drawer_open: bool,
}

impl CadeApp {
    /// Construct from the `CreationContext` handed to us by eframe.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Apply the CADE dark theme once at startup.
        crate::theme::apply_theme(&cc.egui_ctx, &crate::theme::ThemeColors::dark());

        // Resolve the server URL from the page origin.  In production
        // the dashboard is served by cade-server, so origin == API host.
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_else(|| "http://localhost:8284".to_string());
        let query = web_sys::window().and_then(|w| w.location().search().ok());
        let config = Config::resolve(&origin, query.as_deref(), None);

        let mut login = LoginState::new();

        // If a token was previously saved, pre-fill and auto-submit so the
        // first render frame triggers spawn_connect via the existing flow.
        if let Some(saved_token) = crate::storage::load(crate::storage::StorageKey::ApiToken) {
            if !saved_token.is_empty() {
                login.on_input(&saved_token);
                login.on_submit();
            }
        }

        Self {
            login,
            session: Rc::new(RefCell::new(None)),
            connect_started: false,
            ctx: cc.egui_ctx.clone(),
            server_url: config.server_url,
            md_cache: CommonMarkCache::default(),
            input_id: egui::Id::new("chat_input"),
            theme: crate::theme::ThemeColors::dark(),
            viewport: crate::responsive::Viewport::Desktop,
            sidebar_drawer_open: false,
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
        let shortcut = ui.input(poll_shortcut);
        let mut request_focus_input = false;

        // Snapshot session state once so we can read it in the toolbar
        // without holding a borrow into the render closures below.
        let session_snapshot_for_toolbar = self.session.borrow().clone();

        // ── Top toolbar (M1) ─────────────────────────────────────────────
        if components::breadcrumb::render(ui, &session_snapshot_for_toolbar, &self.theme, self.viewport) {
            self.sidebar_drawer_open = !self.sidebar_drawer_open;
        }

        // ── Bottom status bar (M1) ────────────────────────────────────────
        components::footer::render(ui, &session_snapshot_for_toolbar, &self.theme);

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // ── M5: context-window progress bar ──────────────────────────
            if let Some(crate::session::SessionState::Connected {
                total_input_tokens,
                total_output_tokens,
                ..
            }) = session_snapshot_for_toolbar
            {
                const DEFAULT_WINDOW: u64 = 128_000;
                let total = total_input_tokens + total_output_tokens;
                let frac = crate::theme::context_fill_fraction(total, DEFAULT_WINDOW);
                if total > 0 {
                    let bar_color = crate::theme::context_fill_color(frac, &self.theme);
                    let hover_text =
                        format!("{total} / {DEFAULT_WINDOW} tokens ({:.0}%)", frac * 100.0);
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_height(4.0)
                            .fill(bar_color),
                    )
                    .on_hover_text(hover_text);
                }
            }

            // Snapshot session state for this frame's render pass.
            let mut session_snapshot = self.session.borrow().clone();

            match session_snapshot {
                Some(SessionState::Connecting { .. }) => {
                    ui.label("Connecting to server...");
                    ui.spinner();
                }
                Some(SessionState::HealthOk { .. }) => {
                    ui.label("Server reached — loading agents...");
                    ui.spinner();
                }
                Some(SessionState::Connected {
                    ref agents,
                    ref health,
                    ref selected_agent,
                    ref messages,
                    ref input_buffer,
                    streaming,
                    auto_scroll,
                    ref error_toast,
                    ref last_usage,
                    ref last_finish_reason,
                    ref conversations,
                    ref selected_conversation,
                    has_more_messages,
                    palette_open,
                    ref palette_input,
                    palette_selection,
                    menu_open,
                    ref menu_input,
                    menu_selection,
                    memory_open,
                    ref memory_blocks,
                    memory_selection,
                    ref memory_edit_buffer,
                    memory_loading,
                    memory_saving,
                    ref memory_error,
                    ref memory_save_notice,

                    checkpoints_open,
                    ref checkpoints,
                    checkpoints_loading,
                    checkpoints_busy,
                    ref checkpoints_error,
                    ref checkpoints_notice,

                    artifacts_open,
                    ref artifacts,
                    ref artifact_selection,
                    ref artifact_detail,
                    artifacts_loading,
                    artifacts_busy,
                    ref artifacts_error,

                    tools_open,
                    ref tools,
                    tools_loading,
                    ref tools_error,

                    ref active_question,
                    question_cursor,
                    ref question_checked,

                    ref agent_metrics,
                    total_input_tokens,
                    total_output_tokens,

                    context_open,
                    ref context_stats,
                    context_loading,
                    ref context_error,

                    agents_open,
                    stats_open,

                    mcp_open,
                    ref mcp_servers,
                    mcp_loading,
                    ref mcp_error,
                    ref mut theme_update,

                    model_picker_open,
                    ref model_picker_models,
                    ref model_picker_custom_providers,
                    ref model_picker_query,
                    model_picker_selection,
                    model_picker_loading,
                    ref model_picker_error,

                    ref active_plan,
                    ref live_outputs,
                    ref context_breakdown,
                    context_breakdown_loading,

                    providers_open,
                    ref providers,
                    providers_loading,
                    permissions_open,
                    ref current_permission_mode,
                    theme_picker_open,
                    ref available_themes,
                    ref current_theme_name,
                    hooks_open,
                    ref hooks,
                    hooks_loading,
                    toolset_open,
                    ref current_toolset,
                    pricing_open,
                    ref pricing_info,
                    backend_open,
                    ref current_backend,
                    reasoning_open,
                    ref current_reasoning_effort,
                    skills_overlay_open,
                    ref all_skills_list,
                    ref loaded_skill_ids,
                    skills_loading,
                    ref skills_filter,
                    ref subagent_cards,
                    ..
                }) => {
                    // ── Connected: 3-panel layout ───────────────────
                    let _version = health.version.as_deref().unwrap_or("unknown");

                    if let Some(new_theme) = theme_update.take() {
                        self.theme = new_theme;
                        crate::theme::apply_theme(ui.ctx(), &self.theme);
                    }

                    let has_agent = selected_agent.is_some();
                    let is_streaming = streaming;

                    // Clone input buffer for the editable text field.
                    let input_edit = input_buffer.clone();

                    // ── Map keyboard shortcuts and gestures to actions ─────────
                    let gesture = crate::gestures::detect_swipe(ui.ctx());
                    let mut dismiss_gesture = false;
                    if let Some(g) = gesture {
                        match g {
                            crate::gestures::Gesture::SwipeDown | crate::gestures::Gesture::SwipeRight => {
                                dismiss_gesture = true;
                            }
                            crate::gestures::Gesture::SwipeLeft => {
                                // If sidebar drawer is open, close it on swipe left
                                if self.sidebar_drawer_open {
                                    self.sidebar_drawer_open = false;
                                }
                            }
                            _ => {}
                        }
                    }

                    //
                    // When an overlay is open, keys are reinterpreted:
                    //   Palette open:
                    //     Esc      → ClosePalette (overrides DismissError)
                    //     Enter    → ExecutePaletteCmd (overrides Send)
                    //     ArrowUp  → MovePaletteSelection(-1)
                    //     ArrowDown→ MovePaletteSelection(+1)
                    //   Menu overlay open:
                    //     Esc      → CloseMenu
                    //     Enter    → ExecuteMenuCmd (overrides Send)
                    //     ArrowUp  → MoveMenuSelection(-1)
                    //     ArrowDown→ MoveMenuSelection(+1)
                    //   Memory overlay open:
                    //     Esc      → CloseMemoryOverlay
                    //     Ctrl+S   → SaveMemoryBlock (only when dirty)
                    //   Checkpoints overlay open:
                    //     Esc      → CloseCheckpointsOverlay
                    //   Artifacts overlay open:
                    //     Esc      → CloseArtifactsOverlay
                    if palette_open {
                        // Arrow keys aren't in the global SHORTCUTS table,
                        // sample them directly.
                        let (up, down) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::ArrowUp),
                                i.key_pressed(egui::Key::ArrowDown),
                            )
                        });
                        if dismiss_gesture {
                            action = AppAction::ClosePalette;
                        } else if up {
                            action = AppAction::MovePaletteSelection(-1);
                        } else if down {
                            action = AppAction::MovePaletteSelection(1);
                        } else if let Some(sc) = shortcut {
                            match sc {
                                ShortcutAction::DismissError => {
                                    action = AppAction::ClosePalette;
                                }
                                ShortcutAction::Send => {
                                    action = AppAction::ExecutePaletteCmd;
                                }
                                _ => {}
                            }
                        }
                    } else if menu_open {
                        let (up, down) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::ArrowUp),
                                i.key_pressed(egui::Key::ArrowDown),
                            )
                        });
                        if dismiss_gesture {
                            action = AppAction::CloseMenu;
                        } else if up {
                            action = AppAction::MoveMenuSelection(-1);
                        } else if down {
                            action = AppAction::MoveMenuSelection(1);
                        } else if let Some(sc) = shortcut {
                            match sc {
                                ShortcutAction::DismissError => {
                                    action = AppAction::CloseMenu;
                                }
                                ShortcutAction::Send => {
                                    action = AppAction::ExecuteMenuCmd;
                                }
                                _ => {}
                            }
                        }
                    } else if memory_open {
                        // Sample Ctrl+S directly — it isn't in the global
                        // SHORTCUTS table because it is overlay-scoped.
                        let ctrl_s = ui.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.ctrl);
                        let dirty_buf = memory_blocks
                            .get(memory_selection)
                            .is_some_and(|b| b.value != *memory_edit_buffer);
                        if dismiss_gesture {
                            action = AppAction::CloseMemoryOverlay;
                        } else if ctrl_s && dirty_buf && !memory_saving {
                            action = AppAction::SaveMemoryBlock;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseMemoryOverlay;
                        }
                    } else if checkpoints_open {
                        if dismiss_gesture {
                            action = AppAction::CloseCheckpointsOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseCheckpointsOverlay;
                        }
                    } else if artifacts_open {
                        if dismiss_gesture {
                            action = AppAction::CloseArtifactsOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseArtifactsOverlay;
                        }
                    } else if tools_open {
                        if dismiss_gesture {
                            action = AppAction::CloseToolsOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseToolsOverlay;
                        }
                    } else if agents_open {
                        if dismiss_gesture {
                            action = AppAction::CloseAgentsOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseAgentsOverlay;
                        }
                    } else if context_open {
                        if dismiss_gesture {
                            action = AppAction::CloseContextOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseContextOverlay;
                        }
                    } else if stats_open {
                        if dismiss_gesture {
                            action = AppAction::CloseStatsOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseStatsOverlay;
                        }
                    } else if mcp_open {
                        if dismiss_gesture {
                            action = AppAction::CloseMcpOverlay;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseMcpOverlay;
                        }
                    } else if model_picker_open {
                        if dismiss_gesture {
                            action = AppAction::CloseModelPicker;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseModelPicker;
                        }
                    } else if active_question.is_some() {
                        // Inline question widget captures arrow keys + Enter + Esc.
                        let (up, down) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::ArrowUp),
                                i.key_pressed(egui::Key::ArrowDown),
                            )
                        });
                        if up {
                            action = AppAction::MoveQuestionCursor(-1);
                        } else if down {
                            action = AppAction::MoveQuestionCursor(1);
                        } else if let Some(sc) = shortcut {
                            match sc {
                                ShortcutAction::Send => {
                                    action = AppAction::AnswerQuestion;
                                }
                                ShortcutAction::DismissError => {
                                    action = AppAction::DismissQuestion;
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(sc) = shortcut {
                        match sc {
                            ShortcutAction::Send => {
                                if has_agent && !is_streaming && !input_edit.trim().is_empty() {
                                    action = AppAction::SendMessage;
                                }
                            }
                            ShortcutAction::InsertNewline => {
                                // Handled inside TextEdit (multiline future); no-op for singleline.
                            }
                            ShortcutAction::DismissError => {
                                if error_toast.is_some() {
                                    action = AppAction::DismissError;
                                }
                            }
                            ShortcutAction::FocusInput => {
                                request_focus_input = true;
                            }
                            ShortcutAction::OpenPalette => {
                                action = AppAction::OpenPalette(String::new());
                            }
                            // Palette-scoped actions never fire when closed.
                            ShortcutAction::ClosePalette
                            | ShortcutAction::PalettePrev
                            | ShortcutAction::PaletteNext
                            | ShortcutAction::PaletteExecute => {}
                        }
                    }

                    // ── Left sidebar: agent list ────────────────────
                    if let Some(new_action) = components::sidebar::render(
                        ui,
                        &agents,
                        &selected_agent,
                        has_agent,
                        agent_metrics.as_ref(),
                        &conversations,
                        &selected_conversation,
                        is_streaming,
                        active_plan.as_ref(),
                        (total_input_tokens, total_output_tokens),
                        &self.theme,
                        self.viewport,
                        &mut self.sidebar_drawer_open,
                    ) {
                        action = new_action;
                    }

                    // ── Plan panel (inside sidebar, shown when plan active) ──
                    if let Some(plan) = active_plan {
                        // Plan steps already summarized in sidebar Status section.
                        // Full checklist rendered inline below sidebar.
                        if self.viewport.is_desktop() {
                            egui::Panel::left("plan_panel")
                                .default_size(200.0)
                                .resizable(false)
                                .show_inside(ui, |ui| {
                                    components::plan::render(ui, plan, &self.theme);
                                });
                        }
                    }

                    // ── Bottom panel: input bar (TUI-matched) ────────
                    if let Some(new_action) = components::editor::render(
                        ui,
                        input_edit,
                        has_agent,
                        is_streaming,
                        request_focus_input,
                        self.input_id,
                        &self.session,
                        &self.theme,
                    ) {
                        action = new_action;
                    }

                    // ── Central area: timeline ──────────────────────
                    if let Some(new_action) = components::timeline::render(
                        ui,
                        &mut self.md_cache,
                        *selected_agent,
                        &messages,
                        has_more_messages,
                        is_streaming,
                        auto_scroll,
                        error_toast.as_ref(),
                        last_usage.as_ref(),
                        last_finish_reason.as_ref(),
                        live_outputs,
                        subagent_cards,
                        &self.theme,
                    ) {
                        action = new_action;
                    }

                    // ── Slash-command palette overlay ─────────────
                    if palette_open {
                        if let Some(new_action) = render_palette_overlay(
                            ui.ctx(),
                            palette_input,
                            palette_selection,
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Full-Screen Command Menu ─────────────
                    if menu_open {
                        if let Some(new_action) =
                            render_menu_overlay(ui.ctx(), menu_input, menu_selection, &self.theme)
                        {
                            action = new_action;
                        }
                    }

                    // ── Memory-viewer overlay (M16) ───────────────
                    if memory_open {
                        // Derive `dirty` by comparing edit buffer to the
                        // currently-selected block's saved value.  Done
                        // here (not in render fn) so the render fn stays
                        // presentational and easy to unit test.
                        let dirty = memory_blocks
                            .get(memory_selection)
                            .is_some_and(|b| b.value != *memory_edit_buffer);
                        if let Some(new_action) = render_memory_overlay(
                            ui.ctx(),
                            memory_blocks,
                            memory_selection,
                            memory_edit_buffer,
                            memory_loading,
                            memory_saving,
                            memory_error.as_deref(),
                            memory_save_notice.as_deref(),
                            dirty,
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Checkpoints overlay (M17) ─────────────────
                    if checkpoints_open {
                        if let Some(new_action) = render_checkpoints_overlay(
                            ui.ctx(),
                            checkpoints,
                            checkpoints_loading,
                            checkpoints_busy,
                            checkpoints_error.as_deref(),
                            checkpoints_notice.as_deref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Artifacts overlay (M17) ───────────────────
                    if artifacts_open {
                        if let Some(new_action) = render_artifacts_overlay(
                            ui.ctx(),
                            artifacts,
                            *artifact_selection,
                            artifact_detail.as_ref(),
                            artifacts_loading,
                            artifacts_busy,
                            artifacts_error.as_deref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Tools / MCP overlay (M18) ────────────────
                    if tools_open {
                        if let Some(new_action) = render_tools_overlay(
                            ui.ctx(),
                            tools,
                            tools_loading,
                            tools_error.as_deref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Agents overlay (M19) ──────────────────────
                    if agents_open {
                        if let Some(new_action) =
                            render_agents_overlay(ui.ctx(), agents, *selected_agent, &self.theme)
                        {
                            action = new_action;
                        }
                    }

                    // ── Context-stats overlay (M19) ───────────────
                    if context_open {
                        if let Some(new_action) = render_context_overlay(
                            ui.ctx(),
                            context_stats.as_ref(),
                            context_loading,
                            context_error.as_deref(),
                            context_breakdown.as_ref(),
                            context_breakdown_loading,
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Stats overlay (M19) ───────────────────────
                    if stats_open {
                        if let Some(new_action) = render_stats_overlay(
                            ui.ctx(),
                            total_input_tokens,
                            total_output_tokens,
                            last_usage.as_ref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── MCP servers overlay ───────────────────────
                    if mcp_open {
                        if let Some(new_action) = render_mcp_overlay(
                            ui.ctx(),
                            mcp_servers,
                            mcp_loading,
                            mcp_error.as_deref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Model picker overlay ─────────────────────
                    if model_picker_open {
                        if let Some(new_action) = render_model_picker(
                            ui.ctx(),
                            model_picker_models,
                            model_picker_custom_providers,
                            model_picker_query,
                            model_picker_selection,
                            model_picker_loading,
                            model_picker_error.as_deref(),
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Settings overlays ─────────────────────────
                    if providers_open {
                        if let Some(a) = overlays::settings::render_providers_overlay(
                            ui.ctx(),
                            providers,
                            providers_loading,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if permissions_open {
                        if let Some(a) = overlays::settings::render_permissions_overlay(
                            ui.ctx(),
                            current_permission_mode,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if theme_picker_open {
                        if let Some(a) = overlays::settings::render_theme_overlay(
                            ui.ctx(),
                            available_themes,
                            current_theme_name,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if hooks_open {
                        if let Some(a) = overlays::settings::render_hooks_overlay(
                            ui.ctx(),
                            hooks,
                            hooks_loading,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if toolset_open {
                        if let Some(a) = overlays::settings::render_toolset_overlay(
                            ui.ctx(),
                            current_toolset,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if pricing_open {
                        if let Some(a) = overlays::settings::render_pricing_overlay(
                            ui.ctx(),
                            pricing_info,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if backend_open {
                        if let Some(a) = overlays::settings::render_backend_overlay(
                            ui.ctx(),
                            current_backend,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if reasoning_open {
                        if let Some(a) = overlays::settings::render_reasoning_overlay(
                            ui.ctx(),
                            current_reasoning_effort,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }
                    if skills_overlay_open {
                        if let Some(a) = overlays::skills::render_skills_overlay(
                            ui.ctx(),
                            all_skills_list,
                            loaded_skill_ids,
                            skills_loading,
                            skills_filter,
                            &self.theme,
                        ) {
                            action = a;
                        }
                    }

                    // ── Inline question widget (M18) ─────────────
                    if let Some(q) = active_question {
                        if let Some(new_action) = render_question_widget(
                            ui.ctx(),
                            q,
                            question_cursor,
                            question_checked,
                            &self.theme,
                        ) {
                            action = new_action;
                        }
                    }
                }
                Some(SessionState::ConnectionFailed { ref error, .. }) => {
                    ui.colored_label(self.theme.error(), "Connection failed");
                    ui.add_space(4.0);
                    ui.label(error.as_str());
                    ui.add_space(8.0);
                    if ui.button("Retry").clicked() {
                        action = AppAction::Retry;
                    }
                }
                None => {
                    // Still in login flow.
                    match &self.login {
                        LoginState::Entering { buffer } => {
                            ui.label("Paste your CADE API key:");
                            let mut editable = buffer.clone();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut editable)
                                    .password(true)
                                    .desired_width(320.0)
                                    .hint_text("CADE_API_KEY"),
                            );
                            if resp.changed() {
                                self.login.on_input(&editable);
                            }
                            ui.add_space(8.0);
                            let submit_btn = ui.button("Connect");
                            let enter =
                                resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if submit_btn.clicked() || enter {
                                self.login.on_submit();
                            }
                        }
                        LoginState::Submitted { key } => {
                            if !self.connect_started {
                                action = AppAction::Connect(key.clone());
                            }
                            ui.label("Initiating connection...");
                            ui.spinner();
                        }
                    }
                }
            }
        });

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
                    if let Some(SessionState::Connected { input_buffer, .. }) =
                        self.session.borrow_mut().as_mut()
                    {
                        *input_buffer = text;
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
                    if let SessionState::Connected { providers_open, .. } = s {
                        *providers_open = false;
                    }
                }
            }
            AppAction::ClosePermissionsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        permissions_open, ..
                    } = s
                    {
                        *permissions_open = false;
                    }
                }
            }
            AppAction::SetPermissionMode(mode) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        current_permission_mode,
                        permissions_open,
                        ..
                    } = s
                    {
                        *current_permission_mode = mode;
                        *permissions_open = false;
                    }
                }
            }
            AppAction::CloseThemeOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        theme_picker_open, ..
                    } = s
                    {
                        *theme_picker_open = false;
                    }
                }
            }
            AppAction::SetTheme(name) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        current_theme_name,
                        theme_picker_open,
                        ..
                    } = s
                    {
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
                    if let SessionState::Connected { hooks_open, .. } = s {
                        *hooks_open = false;
                    }
                }
            }
            AppAction::CloseToolsetOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { toolset_open, .. } = s {
                        *toolset_open = false;
                    }
                }
            }
            AppAction::SetToolset(ts) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        current_toolset,
                        toolset_open,
                        ..
                    } = s
                    {
                        *current_toolset = ts;
                        *toolset_open = false;
                    }
                }
            }
            AppAction::ClosePricingOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { pricing_open, .. } = s {
                        *pricing_open = false;
                    }
                }
            }
            AppAction::CloseBackendOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { backend_open, .. } = s {
                        *backend_open = false;
                    }
                }
            }
            AppAction::SetBackend(be) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        current_backend,
                        backend_open,
                        ..
                    } = s
                    {
                        *current_backend = be;
                        *backend_open = false;
                    }
                }
            }
            AppAction::CloseReasoningOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { reasoning_open, .. } = s {
                        *reasoning_open = false;
                    }
                }
            }
            AppAction::SetReasoning(level) => {
                let effort_str = level.clone();
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        current_reasoning_effort,
                        reasoning_open,
                        ..
                    } = s
                    {
                        *current_reasoning_effort = level;
                        *reasoning_open = false;
                    }
                }
                self.spawn_patch_reasoning(effort_str);
            }
            AppAction::CloseSkillsOverlay => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected {
                        skills_overlay_open,
                        skills_filter,
                        ..
                    } = s
                    {
                        *skills_overlay_open = false;
                        *skills_filter = String::new();
                    }
                }
            }
            AppAction::SetSkillsFilter(q) => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    if let SessionState::Connected { skills_filter, .. } = s {
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

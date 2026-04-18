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

pub mod overlays;

mod tasks;
mod views;

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
    render_context_overlay, render_memory_overlay, render_model_picker,
    render_palette_overlay, render_question_widget, render_stats_overlay,
    render_tools_overlay,
};
// Bring view helpers into scope.
use views::{render_timeline_message, render_welcome};

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
}

impl CadeApp {
    /// Construct from the `CreationContext` handed to us by eframe.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Apply the CADE dark theme once at startup.
        crate::theme::apply_theme(&cc.egui_ctx);

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
        }
    }

}

impl eframe::App for CadeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Collect actions during rendering so we can apply them after
        // all borrows are released.  This avoids borrow-conflict issues
        // with Rc<RefCell<..>>.
        let mut action = AppAction::None;

        // ── Global keyboard shortcuts ────────────────────────────
        let shortcut = ui.input(poll_shortcut);
        let mut request_focus_input = false;

        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("CADE Dashboard");
            ui.add_space(12.0);

            // Snapshot session state for this frame's render pass.
            let session_snapshot = self.session.borrow().clone();

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
                    ref error_toast,
                    ref last_usage,
                    ref last_finish_reason,
                    ref conversations,
                    ref selected_conversation,
                    has_more_messages,
                    palette_open,
                    ref palette_input,
                    palette_selection,
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

                    model_picker_open,
                    ref model_picker_models,
                    ref model_picker_custom_providers,
                    ref model_picker_query,
                    model_picker_selection,
                    model_picker_loading,
                    ref model_picker_error,
                    ..
                }) => {
                    // ── Connected: 3-panel layout ───────────────────
                    let version = health.version.as_deref().unwrap_or("unknown");
                    let has_agent = selected_agent.is_some();
                    let is_streaming = streaming;

                    // Clone input buffer for the editable text field.
                    let mut input_edit = input_buffer.clone();

                    // ── Map keyboard shortcuts to actions ─────────
                    //
                    // When an overlay is open, keys are reinterpreted:
                    //   Palette open:
                    //     Esc      → ClosePalette (overrides DismissError)
                    //     Enter    → ExecutePaletteCmd (overrides Send)
                    //     ArrowUp  → MovePaletteSelection(-1)
                    //     ArrowDown→ MovePaletteSelection(+1)
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
                        if up {
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
                    } else if memory_open {
                        // Sample Ctrl+S directly — it isn't in the global
                        // SHORTCUTS table because it is overlay-scoped.
                        let ctrl_s = ui.input(|i| {
                            i.key_pressed(egui::Key::S) && i.modifiers.ctrl
                        });
                        let dirty_buf = memory_blocks
                            .get(memory_selection)
                            .is_some_and(|b| b.value != *memory_edit_buffer);
                        if ctrl_s && dirty_buf && !memory_saving {
                            action = AppAction::SaveMemoryBlock;
                        } else if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseMemoryOverlay;
                        }
                    } else if checkpoints_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseCheckpointsOverlay;
                        }
                    } else if artifacts_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseArtifactsOverlay;
                        }
                    } else if tools_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseToolsOverlay;
                        }
                    } else if agents_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseAgentsOverlay;
                        }
                    } else if context_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseContextOverlay;
                        }
                    } else if stats_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
                            action = AppAction::CloseStatsOverlay;
                        }
                    } else if model_picker_open {
                        if let Some(ShortcutAction::DismissError) = shortcut {
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
                    egui::Panel::left("agent_sidebar")
                        .default_size(180.0)
                        .resizable(true)
                        .show_inside(ui, |ui| {
                            ui.heading("Agents");
                            ui.separator();
                            if agents.is_empty() {
                                ui.label("No agents configured.");
                            } else {
                                for (i, agent) in agents.iter().enumerate() {
                                    let is_selected = *selected_agent == Some(i);
                                    let label = format!("🤖 {}", agent.name);
                                    if ui.selectable_label(is_selected, label).clicked()
                                        && !is_selected
                                    {
                                        action = AppAction::SelectAgent(i);
                                    }
                                }
                            }
                            ui.separator();

                            // ── Conversations list ────────────────────
                            if has_agent {
                                // Agent info card — model, provider, id.
                                if let Some(idx) = *selected_agent {
                                    if let Some(agent) = agents.get(idx) {
                                        ui.add_space(2.0);
                                        egui::Frame::new()
                                            .fill(crate::theme::BG_SURFACE0)
                                            .corner_radius(egui::CornerRadius::same(4))
                                            .inner_margin(6.0)
                                            .show(ui, |ui| {
                                                ui.vertical(|ui| {
                                                    if let Some(model) = &agent.model {
                                                        ui.label(
                                                            egui::RichText::new(format!(
                                                                "model: {model}"
                                                            ))
                                                            .monospace()
                                                            .color(crate::theme::PRIMARY)
                                                            .size(11.0),
                                                        );
                                                    }
                                                    if let Some(provider) =
                                                        &agent.provider
                                                    {
                                                        ui.label(
                                                            egui::RichText::new(format!(
                                                                "provider: {provider}"
                                                            ))
                                                            .monospace()
                                                            .color(
                                                                crate::theme::TEXT_MUTED,
                                                            )
                                                            .size(11.0),
                                                        );
                                                    }
                                                    // Show a truncated id so
                                                    // operators can cross-
                                                    // reference with server logs.
                                                    let short_id = if agent.id.len() > 12
                                                    {
                                                        format!("{}…", &agent.id[..12])
                                                    } else {
                                                        agent.id.clone()
                                                    };
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "id: {short_id}"
                                                        ))
                                                        .monospace()
                                                        .color(crate::theme::TEXT_DIM)
                                                        .size(10.0),
                                                    );

                                                    // Metrics — shown when loaded.
                                                    if let Some(m) = agent_metrics {
                                                        ui.add_space(4.0);
                                                        ui.separator();
                                                        for (label, val) in [
                                                            ("consolidations", m.consolidation_runs),
                                                            ("compacted", m.tool_outputs_compacted),
                                                            ("guard hits", m.inflation_guard_hits),
                                                        ] {
                                                            ui.label(
                                                                egui::RichText::new(
                                                                    format!("{label}: {val}"),
                                                                )
                                                                .color(crate::theme::TEXT_DIM)
                                                                .size(10.0),
                                                            );
                                                        }
                                                    }
                                                });
                                            });
                                    }
                                }

                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Conversations")
                                            .strong()
                                            .size(13.0),
                                    );
                                    if ui.small_button("➕ New").clicked() {
                                        action = AppAction::NewConversation;
                                    }
                                });
                                ui.add_space(2.0);

                                if conversations.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No conversations yet.")
                                            .weak()
                                            .size(11.0),
                                    );
                                } else {
                                    for (ci, conv) in conversations.iter().enumerate() {
                                        let is_sel = *selected_conversation == Some(ci);
                                        let title = if conv.title.is_empty() {
                                            "Untitled"
                                        } else {
                                            &conv.title
                                        };
                                        let label = format!(
                                            "💬 {} ({})",
                                            title, conv.message_count
                                        );
                                        if ui
                                            .selectable_label(is_sel, label)
                                            .clicked()
                                            && !is_sel
                                        {
                                            action =
                                                AppAction::SelectConversation(ci);
                                        }
                                    }
                                }
                                ui.separator();
                            }

                            ui.add_space(4.0);
                            if ui.button("🚪 Logout").clicked() {
                                action = AppAction::Logout;
                            }
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("v{version}"))
                                    .small()
                                    .weak(),
                            );
                        });

                    // ── Bottom panel: input bar (TUI-matched) ────────
                    egui::Panel::bottom("input_bar")
                        .min_size(48.0)
                        .show_inside(ui, |ui| {
                            let can_edit = has_agent && !is_streaming;

                            // ── Top separator ─────────────────────────
                            let sep_color = if is_streaming {
                                crate::theme::PRIMARY
                            } else {
                                crate::theme::BORDER_BASE
                            };
                            let sep_rect = ui.available_rect_before_wrap();
                            let sep_rect = egui::Rect::from_min_size(
                                sep_rect.min,
                                egui::vec2(sep_rect.width(), 1.0),
                            );
                            ui.painter().rect_filled(sep_rect, 0.0, sep_color);
                            ui.advance_cursor_after_rect(sep_rect);

                            // ── Input row ─────────────────────────────
                            egui::Frame::new()
                                .fill(crate::theme::BG_INPUT)
                                .inner_margin(egui::Margin::symmetric(8, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        // Mode badge — mirrors TUI's input_mode_badge
                                        let badge_text = if is_streaming {
                                            " WAIT "
                                        } else if !has_agent {
                                            " ··· "
                                        } else {
                                            " CHAT "
                                        };
                                        let badge_bg = if is_streaming {
                                            crate::theme::WARNING
                                        } else {
                                            crate::theme::BG_SURFACE2
                                        };
                                        let badge = egui::RichText::new(badge_text)
                                            .color(crate::theme::TEXT_PRIMARY)
                                            .strong()
                                            .size(11.0)
                                            .background_color(badge_bg);
                                        ui.label(badge);

                                        // Prompt prefix "> "
                                        ui.label(
                                            egui::RichText::new("> ")
                                                .color(crate::theme::TEXT_DIM)
                                                .monospace()
                                                .size(14.0),
                                        );

                                        // Text input — multiline, full width
                                        let hint = if !has_agent {
                                            "Select an agent first…"
                                        } else if is_streaming {
                                            "Waiting for response…"
                                        } else {
                                            "Type a message or paste code…"
                                        };

                                        let desired_w = ui.available_width() - 40.0;
                                        let resp = ui.add_enabled(
                                            can_edit,
                                            egui::TextEdit::multiline(&mut input_edit)
                                                .id(self.input_id)
                                                .hint_text(
                                                    egui::RichText::new(hint)
                                                        .color(crate::theme::TEXT_DIM),
                                                )
                                                .desired_width(desired_w)
                                                .desired_rows(1)
                                                .lock_focus(true)
                                                .font(egui::TextStyle::Monospace),
                                        );

                                        if request_focus_input {
                                            resp.request_focus();
                                        }

                                        if resp.changed() {
                                            let typed_slash_at_start = input_edit.starts_with('/')
                                                && !input_buffer.starts_with('/');
                                            if typed_slash_at_start {
                                                let initial = input_edit
                                                    .strip_prefix('/')
                                                    .unwrap_or("")
                                                    .to_string();
                                                action = AppAction::OpenPalette(initial);
                                                input_edit.clear();
                                            }
                                            if let Some(SessionState::Connected {
                                                input_buffer: buf, ..
                                            }) = self.session.borrow_mut().as_mut()
                                            {
                                                *buf = input_edit.clone();
                                            }
                                        }

                                        // Enter sends (Shift+Enter for newline in multiline)
                                        let enter_pressed = ui.input(|i| {
                                            i.key_pressed(egui::Key::Enter)
                                                && !i.modifiers.shift
                                        }) && resp.has_focus();
                                        let send_enabled =
                                            can_edit && !input_edit.trim().is_empty();

                                        if is_streaming {
                                            ui.spinner();
                                        } else {
                                            // Send button — circular, TUI-matched
                                            let send_btn = egui::Button::new(
                                                egui::RichText::new("↑")
                                                    .color(if send_enabled {
                                                        crate::theme::BG_BASE
                                                    } else {
                                                        crate::theme::TEXT_DIM
                                                    })
                                                    .strong()
                                                    .size(15.0),
                                            )
                                            .fill(if send_enabled {
                                                crate::theme::PRIMARY
                                            } else {
                                                crate::theme::BG_SURFACE2
                                            })
                                            .stroke(egui::Stroke::NONE)
                                            .corner_radius(egui::CornerRadius::same(14))
                                            .min_size(egui::vec2(28.0, 28.0));

                                            if ui
                                                .add_enabled(send_enabled, send_btn)
                                                .on_hover_text("Send (Enter)")
                                                .clicked()
                                                || (enter_pressed && send_enabled)
                                            {
                                                action = AppAction::SendMessage;
                                            }
                                        }
                                    });
                                });

                            // ── Bottom separator ──────────────────────
                            let sep_rect = ui.available_rect_before_wrap();
                            let sep_rect = egui::Rect::from_min_size(
                                sep_rect.min,
                                egui::vec2(sep_rect.width(), 1.0),
                            );
                            ui.painter().rect_filled(sep_rect, 0.0, crate::theme::BORDER_BASE);
                            ui.advance_cursor_after_rect(sep_rect);

                            // ── Footer: model + context info ──────────
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                // "/" palette trigger
                                if has_agent && !is_streaming {
                                    let chip = egui::Button::new(
                                        egui::RichText::new(" / ")
                                            .color(crate::theme::TEXT_DIM)
                                            .monospace()
                                            .size(10.0),
                                    )
                                    .fill(crate::theme::BG_SURFACE1)
                                    .stroke(egui::Stroke::new(
                                        0.5,
                                        crate::theme::BORDER_BASE,
                                    ))
                                    .corner_radius(egui::CornerRadius::same(3));
                                    if ui
                                        .add(chip)
                                        .on_hover_text("Command palette (Ctrl+K)")
                                        .clicked()
                                    {
                                        action = AppAction::OpenPalette(String::new());
                                    }
                                }
                                // Model name
                                if let Some(idx) = selected_agent {
                                    if let Some(agent) = agents.get(*idx) {
                                        if let Some(model_name) = agent.model.as_deref() {
                                            ui.add_space(6.0);
                                            ui.label(
                                                egui::RichText::new(model_name)
                                                    .color(crate::theme::TEXT_DIM)
                                                    .size(10.0),
                                            );
                                        }
                                    }
                                }
                                // Token usage
                                if total_input_tokens + total_output_tokens > 0 {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let total =
                                                total_input_tokens + total_output_tokens;
                                            let label = if total >= 1_000_000 {
                                                format!("{:.1}M tok", total as f64 / 1e6)
                                            } else if total >= 1_000 {
                                                format!("{:.1}K tok", total as f64 / 1e3)
                                            } else {
                                                format!("{total} tok")
                                            };
                                            ui.label(
                                                egui::RichText::new(label)
                                                    .color(crate::theme::TEXT_DIM)
                                                    .size(10.0),
                                            );
                                        },
                                    );
                                }
                            });
                        });

                    // ── Central area: timeline ──────────────────────
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        // Reserve space for the usage footer before the
                        // scroll area so it doesn't overlap content.
                        let footer_h = if last_usage.is_some() { 22.0 } else { 0.0 };
                        let toast_h = if error_toast.is_some() { 42.0 } else { 0.0 };
                        let reserved = footer_h + toast_h + 4.0;
                        let avail_h = (ui.available_height() - reserved).max(60.0);

                        egui::ScrollArea::vertical()
                            .id_salt("timeline_scroll")
                            .stick_to_bottom(true)
                            .max_height(avail_h)
                            .show(ui, |ui| {
                                // 16px left indent — matches TUI's indent style.
                                let pad = 16.0;
                                ui.add_space(4.0);

                                if selected_agent.is_none() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(pad);
                                        ui.vertical(|ui| {
                                            render_welcome(ui, &mut self.md_cache);
                                        });
                                    });
                                } else if messages.is_empty() && !is_streaming {
                                    ui.add_space(24.0);
                                    ui.horizontal(|ui| {
                                        ui.add_space(pad);
                                        ui.label(
                                            egui::RichText::new("No messages yet. Send one to start a conversation.")
                                                .color(crate::theme::TEXT_MUTED)
                                                .italics()
                                                .size(12.0),
                                        );
                                    });
                                } else {
                                    // ── Load-more ──────────────────────
                                    if has_more_messages {
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            if ui.add(
                                                egui::Button::new(
                                                    egui::RichText::new("⬆  Load older messages")
                                                        .color(crate::theme::TEXT_MUTED)
                                                        .size(11.0),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::new(1.0, crate::theme::BORDER_BASE))
                                            ).clicked() {
                                                action = AppAction::LoadMore;
                                            }
                                        });
                                        ui.add(egui::Separator::default().horizontal().spacing(4.0));
                                    }

                                    for (i, msg) in messages.iter().enumerate() {
                                        // Dim separator between messages
                                        if i > 0 {
                                            ui.horizontal(|ui| {
                                                ui.add_space(pad);
                                                ui.add(egui::Separator::default().horizontal().spacing(2.0));
                                            });
                                        }
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            ui.vertical(|ui| {
                                                if let Some(a) = render_timeline_message(
                                                    ui,
                                                    &mut self.md_cache,
                                                    msg,
                                                ) {
                                                    action = a;
                                                }
                                            });
                                        });
                                    }

                                    // ── Streaming indicator ─────────────
                                    if is_streaming {
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            ui.add(egui::Separator::default().horizontal().spacing(2.0));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            ui.label(
                                                egui::RichText::new("▍ CADE")
                                                    .color(crate::theme::PRIMARY)
                                                    .strong()
                                                    .size(13.0),
                                            );
                                            ui.add_space(6.0);
                                            ui.spinner();
                                        });
                                    }
                                    ui.add_space(8.0);
                                }
                            });

                        // ── Usage stats footer ─────────────────────
                        if let Some((inp, out, model)) = last_usage {
                            ui.add_space(2.0);
                            let model_str = model
                                .as_ref()
                                .map(|m| format!(" · {m}"))
                                .unwrap_or_default();
                            let finish = last_finish_reason
                                .as_ref()
                                .map(|r| format!(" · {r}"))
                                .unwrap_or_default();
                            ui.label(
                                egui::RichText::new(format!(
                                    "↑{inp} ↓{out} tokens{model_str}{finish}"
                                ))
                                .color(crate::theme::TEXT_DIM)
                                .size(11.0),
                            );
                        }

                        // ── Error toast ────────────────────────────
                        if let Some(err) = error_toast {
                            ui.add_space(4.0);
                            egui::Frame::new()
                                .fill(crate::theme::ERROR.gamma_multiply(0.12))
                                .stroke(egui::Stroke::new(1.0, crate::theme::ERROR))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("⚠")
                                                .color(crate::theme::ERROR)
                                                .strong(),
                                        );
                                        ui.label(
                                            egui::RichText::new(err.as_str())
                                                .color(crate::theme::TEXT_PRIMARY)
                                                .size(12.0),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("✕").clicked() {
                                                action = AppAction::DismissError;
                                            }
                                        });
                                    });
                                });
                        }
                    });

                    // ── Slash-command palette overlay ─────────────
                    if palette_open {
                        if let Some(new_action) = render_palette_overlay(
                            ui.ctx(),
                            palette_input,
                            palette_selection,
                        ) {
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
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Agents overlay (M19) ──────────────────────
                    if agents_open {
                        if let Some(new_action) = render_agents_overlay(
                            ui.ctx(),
                            agents,
                            *selected_agent,
                        ) {
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
                        ) {
                            action = new_action;
                        }
                    }

                    // ── Inline question widget (M18) ─────────────
                    if let Some(q) = active_question {
                        if let Some(new_action) = render_question_widget(
                            ui.ctx(),
                            q,
                            question_cursor,
                            question_checked,
                        ) {
                            action = new_action;
                        }
                    }
                }
                Some(SessionState::ConnectionFailed { ref error, .. }) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 50, 50),
                        "Connection failed",
                    );
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
                            let enter = resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
            AppAction::NewConversation => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.on_new_conversation();
                }
            }
            AppAction::LoadMore => self.spawn_load_more_messages(),
            AppAction::Logout => self.logout(),
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
                let answer = self
                    .session
                    .borrow_mut()
                    .as_mut()
                    .and_then(|s| {
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
    /// User clicked "New Chat" — start a fresh conversation.
    NewConversation,
    /// User clicked "Load more" to fetch older messages.
    LoadMore,
    /// User clicked Logout — clear credentials and return to login.
    Logout,
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
    /// Close the model picker overlay.
    CloseModelPicker,
    /// Update model picker search query.
    SetModelPickerQuery(String),
    /// Move model picker selection to index.
    SetModelPickerSelection(usize),
    /// User selected a model from the picker — apply it.
    SelectModel(String),
}

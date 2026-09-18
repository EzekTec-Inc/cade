use crate::app::layout::helpers::{
    abbreviate_cwd, format_token_count, mode_footer_left, mode_sep_color, truncate_str,
};
use crate::colors::ThemeColorsExt;

/// Pick the animated spinner color for the current elapsed ms.
/// Cycles through the theme's 4-step spinner gradient.
fn spinner_color(ms: u128, colors: &ThemeColors) -> RC {
    let palette = [
        colors.c_spinner_0(),
        colors.c_spinner_1(),
        colors.c_spinner_2(),
        colors.c_spinner_3(),
    ];
    palette[(ms / 400) as usize % palette.len()]
}

// Spinner frames for the inline working/thinking status line.  Reuses the
// braille/blocksy cycles the removed bottom status bar used so the live
// state indicator animates visibly instead of freezing on one glyph.
const STATUS_BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const STATUS_DOTS: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Animate the ephemeral working/thinking status line rendered inline at the
/// bottom of the timeline.  A rotating spinner glyph (driven by the wall
/// clock) is prepended on every frame so the status visibly moves while the
/// agent is busy; terminal states (`✓`, `✗`, `⚠`) keep their own static
/// glyphs.  The `● ` bullet used by tool-running statuses is replaced by the
/// spinner so the whole line animates rather than freezing.
fn animate_live_status(text: &str, elapsed: Option<std::time::Duration>) -> String {
    if text.starts_with('✓') || text.starts_with('✗') || text.starts_with('⚠') {
        return text.to_string();
    }
    let frame = match elapsed {
        Some(e) => {
            let ms = e.as_millis();
            if (ms / 3000) % 2 == 0 {
                STATUS_BRAILLE[(ms / 80) as usize % STATUS_BRAILLE.len()]
            } else {
                STATUS_DOTS[(ms / 100) as usize % STATUS_DOTS.len()]
            }
        }
        None => "●",
    };
    let body = text.strip_prefix("● ").unwrap_or(text);
    format!("{frame} {body}")
}

// Rendering helpers for the TuiApp full-screen layout.
//
// Contains `render_frame` and all supporting free functions for drawing
// the conversation timeline, question panel, picker overlay, footer, etc.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::colors::ThemeColors;
use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

use super::layout::cursor::{calc_input_rows, input_mode_badge, rendered_textarea_cursor_position};
use super::layout::sidebar::{SidebarState, render_sidebar};
use super::layout::toast::render_toast;
use super::timeline::{
    TimelineKey, TimelineLayoutEngine, build_timeline_entries, render_timeline_viewport,
};
use super::{
    FIXED_ROWS, MAX_INPUT_ROWS, PlanState, RenderLine, SIDEBAR_BREAKPOINT, SIDEBAR_WIDTH, Toast,
};

// -- Scroll helpers

/// Count the number of visual (terminal) rows a single `Line` occupies when
/// word-wrapped to `content_w` columns.  Uses unicode display-width so emoji
/// and CJK characters are measured correctly.
/// Matches ratatui's `WordWrapper` behaviour: words are broken on whitespace;
/// a word that would overflow the current row starts a new row.
pub(crate) fn count_wrapped_rows(line: &Line<'_>, content_w: u16) -> u16 {
    if content_w == 0 {
        return 1;
    }
    // Concatenate all spans into a single string for word counting.
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if text.is_empty() {
        return 1;
    }
    // V-03: split on \n first — each newline forces a new visual row regardless
    // of wrapping, matching ratatui's behaviour for embedded newlines in spans.
    text.split('\n')
        .map(|segment| count_wrapped_segment(segment, content_w))
        .sum::<u16>()
        .max(1)
}

/// Count wrapped rows for a single line segment (no embedded newlines).
pub(crate) fn count_wrapped_segment(text: &str, content_w: u16) -> u16 {
    if text.is_empty() {
        return 1;
    }
    let width = content_w as usize;
    if width == 0 {
        return 1;
    }
    let mut rows: u16 = 1;
    let mut row_w: usize = 0;
    // split_inclusive preserves the trailing space/tab on each "word" token,
    // which keeps the total width calculation correct.
    for word in text.split_inclusive([' ', '\t']) {
        let word_w = UnicodeWidthStr::width(word);
        if row_w > 0 && row_w + word_w > width {
            rows += 1;
            row_w = 0;
        }

        if word_w > width {
            // A single word is longer than the width. Ratatui will wrap it
            // across multiple lines.
            let extra_rows = (word_w.saturating_sub(1)) / width;
            rows += extra_rows as u16;
            row_w = word_w - (extra_rows * width);
        } else {
            row_w += word_w;
        }
    }
    rows
}

// -- Frame renderer

pub(crate) struct RenderContext<'a> {
    pub(crate) lines: &'a [RenderLine],
    pub(crate) streaming: Option<&'a str>,
    pub(crate) reasoning: Option<&'a str>,
    pub(crate) scroll: usize,
    pub(crate) expand_all: bool,
    pub(crate) input_mode: InputMode,
    pub(crate) mode: PermissionMode,
    pub(crate) agent_name: &'a str,
    pub(crate) model: &'a str,
    pub(crate) last_status: &'a Option<String>,
    pub(crate) thinking_text: Option<&'a str>,
    pub(crate) thinking_elapsed: Option<std::time::Duration>,
    pub(crate) top_overlay: Option<&'a dyn crate::overlay_component::OverlayComponent>,
    pub(crate) queued_count: usize,
    pub(crate) cwd: &'a str,
    pub(crate) context_pct: Option<u8>,
    pub(crate) session_tokens: (u64, u64),
    pub(crate) session_cost_usd: f64,
    pub(crate) session_cost_cap_usd: f64,
    pub(crate) turn_count: u32,
    pub(crate) token_history: &'a [u8],
    pub(crate) header_lines: &'a [RenderLine],
    pub(crate) footer_extra: Option<&'a str>,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) active_plan: Option<&'a PlanState>,
    pub(crate) sidebar_hidden: bool,
    pub(crate) toast: Option<&'a Toast>,
    pub(crate) is_processing: bool,
    pub(crate) copy_highlight: Option<(usize, std::time::Instant)>,
    pub(crate) mouse_selection: Option<usize>,
    pub(crate) expanded_items: &'a std::collections::HashSet<TimelineKey>,
    pub(crate) colors: &'a ThemeColors,
    pub(crate) nerd: bool,
    pub(crate) subagent_trackers: &'a [crate::subagent_tracker::SubagentTracker],
    pub(crate) content_version: u64,
    pub(crate) modified_files: &'a [crate::app::layout::modified_files::ModifiedFileEntry],
    pub(crate) streaming_metrics: Option<crate::app::StreamingMetrics>,
    pub(crate) proxy_status: Option<&'a str>,
    pub(crate) subagent_tray: Option<&'a crate::app::subagent_tray::SubagentTrayState>,
}

pub(crate) fn render_frame(
    frame: &mut Frame,
    ctx: RenderContext<'_>,
    textarea: &mut tui_textarea::TextArea<'static>,
    last_input_width: &mut u16,
    layout_engine: &mut TimelineLayoutEngine,
) -> (u16, Option<(u16, u16)>, ratatui::layout::Rect) {
    let RenderContext {
        lines,
        streaming,
        reasoning,
        scroll,
        expand_all,
        mode,
        agent_name,
        model,
        cwd,
        context_pct,
        turn_count,
        token_history,
        header_lines,
        footer_extra,
        reasoning_effort,
        active_plan,
        toast,
        copy_highlight,
        mouse_selection,
        expanded_items,
        colors,
        nerd,
        subagent_trackers,
        modified_files,
        content_version,
        ..
    } = ctx;

    // returns max_skip for V-04 scroll clamping + messages_area for click-to-copy
    let area = frame.area();
    if area.width < 40 || area.height < 10 {
        render_fallback_too_small(frame, area, colors);
        return (0, None, ratatui::layout::Rect::default());
    }

    let (main_area, sidebar_area) = if area.width >= SIDEBAR_BREAKPOINT && !ctx.sidebar_hidden {
        let sidebar_w = SIDEBAR_WIDTH.min(area.width.saturating_sub(24));
        let split =
            Layout::horizontal([Constraint::Min(24), Constraint::Length(sidebar_w)]).split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };

    let (content_area, subagent_tray_area) = if let Some(tray) = ctx.subagent_tray {
        if tray.is_visible && main_area.width >= 100 {
            let split = Layout::horizontal([
                Constraint::Percentage(65),
                Constraint::Percentage(35),
            ])
            .split(main_area);
            (split[0], Some(split[1]))
        } else {
            (main_area, None)
        }
    } else {
        (main_area, None)
    };

    let w = content_area.width as usize;

    let input = textarea.lines().join("\n");
    let (input_badge, _input_badge_color) = input_mode_badge(ctx.input_mode, colors);
    let input_prefix_w = input_badge.chars().count() as u16 + 1 + 2;
    let available_w = content_area.width;
    let inline_h = ctx
        .top_overlay
        .map(|o| o.inline_height(content_area.height))
        .unwrap_or(0);
    let mut input_rows =
        calc_input_rows(&input, available_w, input_prefix_w).clamp(1, MAX_INPUT_ROWS);

    if inline_h > 0 {
        input_rows = inline_h;
    }

    // A-02: footer_extra adds one row below the normal footer when present.
    let footer_extra_h: u16 = if footer_extra.is_some() { 1 } else { 0 };
    let hotkey_bar_h: u16 = 1;
    let bottom_rows = FIXED_ROWS + input_rows + 2 + footer_extra_h + hotkey_bar_h;

    if content_area.height <= bottom_rows + 1 {
        frame.render_widget(
            Paragraph::new("Terminal too small").style(colors.error()),
            content_area,
        );
        return (0, None, ratatui::layout::Rect::default());
    }

    let plan_h = if let Some(plan) = active_plan {
        if plan.is_visible {
            (plan.steps.len() as u16 + 2).min(10).max(4)
        } else {
            0
        }
    } else {
        0
    };

    let chunks = Layout::vertical([
        Constraint::Fill(1),                                   // [0] content  (fluid)
        Constraint::Length(plan_h),                            // [1] plan panel (0 when hidden)
        Constraint::Length(input_rows + 2),                    // [2] floating rounded input box
        Constraint::Length(1 + footer_extra_h + hotkey_bar_h), // [3] footer
    ])
    .split(content_area);

    // -- Pinned header & viewport layout splits
    let (header_area_opt, messages_area) =
        render_pinned_header(frame, chunks[0], header_lines, w, colors, nerd);
    let _ = header_area_opt;

    // -- Content area
    let timeline_w = messages_area.width.saturating_sub(4).max(1) as usize;
    // Live thinking/working status is rendered as the bottom-most timeline
    // entry (replacing the removed bottom status bar), always pinned below
    // whatever content is currently streaming.  The spinner glyph is animated
    // per frame so the working state visibly moves while the agent is busy.
    let live_status: Option<String> = if let Some(t) = ctx.thinking_text {
        Some(animate_live_status(t, ctx.thinking_elapsed))
    } else {
        ctx.last_status.as_deref().map(str::to_string)
    };
    layout_engine.set_processing(ctx.is_processing);
    layout_engine.set_active_stream(streaming);
    layout_engine.set_active_reasoning(reasoning);
    layout_engine.set_active_status(live_status.as_deref());
    let prepared = layout_engine.layout_items(
        lines,
        timeline_w,
        expand_all,
        expanded_items,
        colors,
        nerd,
        content_version,
    );

    let max_skip = render_timeline_viewport(
        frame,
        messages_area,
        prepared,
        scroll,
        colors,
        copy_highlight,
        mouse_selection,
    );

    // -- Input area or Question Panel (floating rounded container with embedded status pills)
    let input_cursor_pos =
        render_input_or_question(frame, chunks[2], textarea, last_input_width, &ctx, colors);

    // -- Footer bars & Hotkeys
    render_footer_bars(frame, chunks[3], &ctx, footer_extra_h, colors);

    // -- Sidebar
    if let Some(sidebar) = sidebar_area {
        let sidebar_state = SidebarState {
            mode,
            input_mode: ctx.input_mode,
            agent_name,
            model,
            reasoning_effort,
            cwd,
            context_pct,
            turn_count,
            token_history,
            queued_count: ctx.queued_count,
            thinking_text: ctx.thinking_text,
            thinking_elapsed: ctx.thinking_elapsed,
            active_plan,
            session_cost_usd: ctx.session_cost_usd,
            session_cost_cap_usd: ctx.session_cost_cap_usd,
            modified_files,
            streaming_metrics: ctx.streaming_metrics,
            proxy_status: ctx.proxy_status,
        };
        render_sidebar(frame, sidebar, &sidebar_state, colors);
    }

    // -- Toast notifications: suppressed while CADE is processing or working on a task
    if !ctx.is_processing
        && let Some(toast) = toast
    {
        render_toast(frame, main_area, toast, colors);
    }

    // -- Todos / Active Plan checklist
    if let Some(plan) = active_plan
        && plan.is_visible
    {
        render_active_plan(frame, chunks[1], plan, colors);
    }

    // -- Subagent Control Tray
    if let Some(tray) = ctx.subagent_tray
        && tray.is_visible
    {
        if let Some(tray_rect) = subagent_tray_area {
            tray.render(frame, tray_rect, subagent_trackers, colors);
        } else {
            let overlay_rect = ratatui::layout::Rect {
                x: main_area.x + 1,
                y: main_area.y + 1,
                width: main_area.width.saturating_sub(2),
                height: main_area.height.saturating_sub(2),
            };
            frame.render_widget(ratatui::widgets::Clear, overlay_rect);
            tray.render(frame, overlay_rect, subagent_trackers, colors);
        }
    }

    // -- Parallel & Subagent Concurrent Task Matrix (active when tray is closed)
    if !subagent_trackers.is_empty()
        && ctx.subagent_tray.map(|t| !t.is_visible).unwrap_or(true)
    {
        render_subagent_task_matrix(frame, content_area, subagent_trackers, colors);
    }

    (max_skip, input_cursor_pos, messages_area)
}

// ── Sectional Rendering Helpers ──────────────────────────────────────────────

fn render_fallback_too_small(frame: &mut Frame, area: ratatui::layout::Rect, colors: &ThemeColors) {
    use crate::colors::ThemeColorsExt;
    use ratatui::layout::Alignment;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(colors.border_muted())
        .style(colors.style_base());
    let msg = if area.width >= 34 {
        "Resize terminal to render CADE"
    } else if area.width >= 20 {
        "Resize terminal"
    } else {
        "..."
    };
    let paragraph = Paragraph::new(msg)
        .alignment(Alignment::Center)
        .block(block)
        .style(colors.text_dim());
    frame.render_widget(paragraph, area);
}

fn render_pinned_header(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    header_lines: &[RenderLine],
    w: usize,
    colors: &ThemeColors,
    nerd: bool,
) -> (Option<ratatui::layout::Rect>, ratatui::layout::Rect) {
    let content_w = area.width.max(1);
    let mut header_text: Vec<Line<'static>> = Vec::new();
    for entry in build_timeline_entries(header_lines) {
        entry.render_into(w, false, &mut header_text, colors, nerd);
    }
    if header_text.is_empty() {
        (None, area)
    } else {
        let hh: u16 = header_text
            .iter()
            .map(|l| count_wrapped_rows(l, content_w))
            .sum::<u16>()
            .min(area.height / 3)
            .max(1);
        let split = Layout::vertical([Constraint::Length(hh), Constraint::Min(0)]).split(area);
        frame.render_widget(
            Paragraph::new(header_text).wrap(Wrap { trim: false }),
            split[0],
        );
        (Some(split[0]), split[1])
    }
}

#[allow(dead_code)]
fn render_input_separator(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    ctx: &RenderContext<'_>,
    colors: &ThemeColors,
) {
    if area.height == 0 {
        return;
    }
    let RenderContext {
        mode,
        thinking_elapsed,
        streaming,
        ..
    } = ctx;

    let mode_color = mode_sep_color(*mode, colors);
    let top_sep_color = if let Some(elapsed) = thinking_elapsed {
        let ms = elapsed.as_millis();
        spinner_color(ms, colors)
    } else if streaming.is_some() {
        colors.c_primary()
    } else {
        mode_color
    };
    let sep = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep, Style::default().fg(top_sep_color))),
        area,
    );
}

fn render_input_or_question(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    textarea: &mut tui_textarea::TextArea<'static>,
    last_input_width: &mut u16,
    ctx: &RenderContext<'_>,
    colors: &ThemeColors,
) -> Option<(u16, u16)> {
    let RenderContext {
        input_mode,
        queued_count,
        top_overlay,
        cwd,
        ..
    } = ctx;

    let inline_h = top_overlay
        .map(|o| o.inline_height(frame.area().height))
        .unwrap_or(0);

    if inline_h > 0 {
        if let Some(top) = top_overlay {
            top.render_inline(frame, area, colors);
        }
        None
    } else {
        frame.render_widget(ratatui::widgets::Clear, area);

        // 1. Determine mode badge for title_top
        let (badge_text, badge_color) = input_mode_badge(*input_mode, colors);
        let mode_label = format!("● {badge_text}");
        let title_top = Line::from(vec![
            Span::raw(" "),
            Span::styled(
                format!(" {mode_label} "),
                Style::default()
                    .fg(colors.c_bg_base())
                    .bg(badge_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]);

        // 2. Build bottom status pills: File path, Queued
        let mut bottom_pills: Vec<Span<'static>> = Vec::new();
        bottom_pills.push(Span::raw(" "));

        // File path pill (abbreviated to the last 2 path components).
        let path_display = truncate_str(&abbreviate_cwd(std::path::Path::new(cwd)), 26);
        bottom_pills.push(Span::styled(
            format!(" [{path_display}] "),
            Style::default()
                .fg(colors.c_text_muted())
                .add_modifier(Modifier::DIM),
        ));

        // Queued badge
        if *queued_count > 0 {
            bottom_pills.push(Span::styled(
                format!(" [{queued_count} queued] "),
                Style::default()
                    .fg(colors.c_warning())
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let title_bottom = Line::from(bottom_pills);

        // 3. Floating rounded block
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(colors.c_border_muted()))
            .title(title_top)
            .title_bottom(title_bottom);

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        if inner_area.width == 0 || inner_area.height == 0 {
            return None;
        }

        // 4. Prefix "> " and textarea inside floating rounded container
        let prefix_w = 2u16;
        let input_chunks = Layout::horizontal([Constraint::Length(prefix_w), Constraint::Fill(1)])
            .split(inner_area);

        frame.render_widget(
            Paragraph::new(Span::styled(
                "> ",
                colors.primary().add_modifier(Modifier::BOLD),
            )),
            input_chunks[0],
        );

        let input_placeholder = if !textarea.is_empty() {
            String::new()
        } else if *queued_count > 0 {
            format!("{queued_count} queued — type another or Ctrl+Enter to redirect")
        } else {
            "Type a message, @-file, or /command…".to_string()
        };

        textarea.set_placeholder_text(input_placeholder);
        textarea.set_placeholder_style(colors.text_muted());
        textarea.set_cursor_line_style(Style::default());
        let cursor_style = Style::default()
            .fg(colors.c_bg_base())
            .bg(colors.c_primary());
        textarea.set_cursor_style(cursor_style);
        textarea.set_style(Style::default());

        frame.render_widget(&*textarea, input_chunks[1]);

        let rendered_cursor =
            rendered_textarea_cursor_position(&*textarea, input_chunks[1], cursor_style);
        let (visual_x, relative_visual_y) = rendered_cursor.unwrap_or_else(|| {
            let input = textarea.lines().join("\n");
            let (visual_x, visual_y) = super::layout::cursor::calc_visual_cursor(
                &input,
                textarea.cursor().0,
                textarea.cursor().1,
                input_chunks[1].width,
                0,
            );

            let relative_visual_y = if visual_y >= input_chunks[1].height {
                let scroll_top = visual_y
                    .saturating_sub(input_chunks[1].height)
                    .saturating_add(1);
                visual_y.saturating_sub(scroll_top)
            } else {
                visual_y
            };

            (visual_x, relative_visual_y)
        });

        *last_input_width = input_chunks[1].width;

        Some((
            input_chunks[1].x + visual_x,
            input_chunks[1].y + relative_visual_y,
        ))
    }
}

fn render_footer_bars(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    ctx: &RenderContext<'_>,
    footer_extra_h: u16,
    colors: &ThemeColors,
) {
    let RenderContext {
        mode,
        agent_name,
        model,
        reasoning_effort,
        context_pct,
        session_tokens,
        footer_extra,
        top_overlay,
        streaming,
        ..
    } = ctx;

    let (left_label, left_glyph, left_color) = mode_footer_left(*mode, colors);
    let right_agent = agent_name.to_string();
    let right_model = format!(" [{}]", truncate_str(model, 30));
    let right_reasoning = reasoning_effort
        .map(|r| format!(" [{r}]"))
        .unwrap_or_default();
    let (right_ctx, right_ctx_color) = match context_pct {
        Some(p) if *p >= 90 => (format!(" {p}%"), colors.c_error()),
        Some(p) if *p >= 80 => (format!(" {p}%"), colors.c_warning()),
        Some(p) => (format!(" {p}%"), colors.c_text_muted()),
        None => (String::new(), colors.c_text_muted()),
    };
    let right_tokens = if *session_tokens == (0, 0) {
        String::new()
    } else {
        let total = session_tokens.0 + session_tokens.1;
        format!(" {}↑", format_token_count(total))
    };

    let left_base_len: u16 = left_label.chars().count() as u16
        + if left_glyph.is_empty() {
            0
        } else {
            1 + left_glyph.chars().count() as u16
        };
    let right_fixed_len: u16 = (right_agent.chars().count()
        + right_model.chars().count()
        + right_reasoning.chars().count()
        + right_ctx.chars().count()
        + right_tokens.chars().count()) as u16;
    let pad = area.width.saturating_sub(left_base_len + right_fixed_len) as usize;

    let mut footer: Vec<Span<'static>> = vec![Span::styled(
        left_label,
        Style::default().fg(left_color).add_modifier(Modifier::BOLD),
    )];
    if !left_glyph.is_empty() {
        footer.push(Span::styled(
            format!(" {left_glyph}"),
            Style::default().fg(left_color),
        ));
    }
    footer.push(Span::raw(" ".repeat(pad)));
    if !right_agent.is_empty() {
        footer.push(Span::styled(right_agent, colors.thinking_minimal()));
    }
    if !right_model.is_empty() {
        footer.push(Span::styled(right_model, colors.text_dim()));
    }
    if !right_reasoning.is_empty() {
        footer.push(Span::styled(right_reasoning, colors.warning()));
    }
    if !right_ctx.is_empty() {
        footer.push(Span::styled(
            right_ctx,
            Style::default().fg(right_ctx_color),
        ));
    }
    if !right_tokens.is_empty() {
        footer.push(Span::styled(right_tokens, colors.text_dim()));
    }

    let footer_base_rect = ratatui::layout::Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(footer)), footer_base_rect);

    if let Some(extra) = footer_extra {
        let extra_rect = ratatui::layout::Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_str(extra, extra_rect.width.saturating_sub(1) as usize),
                colors.text_dim(),
            )),
            extra_rect,
        );
    }

    let hotkey_rect = ratatui::layout::Rect {
        x: area.x,
        y: area.y + 1 + footer_extra_h,
        width: area.width,
        height: 1,
    };
    let hotkey_spans = if top_overlay.is_some() {
        vec![
            Span::styled(" ▲▼ ", colors.primary_bold()),
            Span::styled("Navigate", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" ↵ ", colors.primary_bold()),
            Span::styled("Select", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" Esc ", colors.primary_bold()),
            Span::styled("Close", colors.text_muted()),
        ]
    } else if streaming.is_some() {
        vec![
            Span::styled(" ^C ", colors.primary_bold()),
            Span::styled("Abort Stream", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" Space ", colors.primary_bold()),
            Span::styled("Snap Bottom", colors.text_muted()),
        ]
    } else {
        vec![
            Span::styled(" ^X ", colors.primary_bold()),
            Span::styled("Leader", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" ^B ", colors.primary_bold()),
            Span::styled("Sidebar", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" ^O ", colors.primary_bold()),
            Span::styled("Expand All", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" Tab ", colors.primary_bold()),
            Span::styled("Cycle Mode", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" ^P ", colors.primary_bold()),
            Span::styled("Menu", colors.text_muted()),
            Span::styled("  │  ", colors.text_dim()),
            Span::styled(" ? ", colors.primary_bold()),
            Span::styled("Help", colors.text_muted()),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(hotkey_spans)), hotkey_rect);
}

fn render_active_plan(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    plan: &PlanState,
    colors: &ThemeColors,
) {
    use ratatui::widgets::{
        List, ListItem, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState,
    };

    let completed_count = plan.steps.iter().filter(|s| s.is_done).count();
    let total_steps = plan.steps.len();

    let mut items = Vec::new();
    for step in &plan.steps {
        let (prefix, prefix_style, text_style) = if step.is_done {
            (
                "✓ ",
                Style::default()
                    .fg(colors.c_success())
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(colors.c_text_muted()),
            )
        } else {
            (
                "● ",
                Style::default()
                    .fg(colors.c_border_accent())
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(colors.c_text_primary())
                    .add_modifier(Modifier::BOLD),
            )
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!(" {prefix}"), prefix_style),
            Span::styled(format!("{}. ", step.id), colors.text_muted()),
            Span::styled(step.description.clone(), text_style),
        ])));
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
    let needs_scrollbar = total_steps > visible_rows;

    let title_str = format!(" Tasks ({completed_count} of {total_steps} completed) ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .title(title_str)
        .title_style(colors.primary_bold())
        .border_style(colors.border_base());

    let list = List::new(items).block(block);
    let mut list_state = ListState::default().with_offset(plan.scroll_offset);
    frame.render_stateful_widget(list, area, &mut list_state);

    if needs_scrollbar {
        let mut scrollbar_state = ScrollbarState::new(total_steps.saturating_sub(visible_rows))
            .position(plan.scroll_offset);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        frame.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_subagent_task_matrix(
    frame: &mut Frame,
    main_area: ratatui::layout::Rect,
    trackers: &[crate::subagent_tracker::SubagentTracker],
    colors: &ThemeColors,
) {
    if trackers.is_empty() {
        return;
    }

    use ratatui::widgets::Clear;

    let active_count = trackers
        .iter()
        .filter(|t| matches!(t.status, crate::subagent_tracker::SubagentStatus::Running))
        .count();
    let completed_count = trackers.len().saturating_sub(active_count);

    let title = format!(
        " ⠋ Concurrent Task Matrix ({} active, {} completed) ",
        active_count, completed_count
    );

    let max_w = 64.min(main_area.width.saturating_sub(4));
    let width = max_w.max(36);
    let row_count = trackers.len();
    let height = (row_count as u16 + 2).min(main_area.height.saturating_sub(2));

    let x = main_area.x + main_area.width.saturating_sub(width + 2);
    let y = main_area.y + 1;

    let rect = ratatui::layout::Rect::new(x, y, width, height);

    let shadow_rect = ratatui::layout::Rect::new(x + 1, y + 1, width, height);
    let shadow_text = vec![
        Line::from(Span::styled(
            "█".repeat(width as usize),
            Style::default().fg(colors.c_text_dim()),
        ));
        height as usize
    ];
    frame.render_widget(Paragraph::new(shadow_text), shadow_rect);

    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .style(colors.style_surface1())
        .border_style(colors.primary())
        .title(Span::styled(
            title,
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut rows = Vec::new();
    let val_w = inner.width as usize;

    for tracker in trackers.iter().take(inner.height as usize) {
        let (glyph, glyph_color) = match tracker.status {
            crate::subagent_tracker::SubagentStatus::Running => ("● ", colors.c_warning()),
            crate::subagent_tracker::SubagentStatus::Completed { .. } => ("✓ ", colors.c_success()),
            crate::subagent_tracker::SubagentStatus::Failed { .. } => ("✗ ", colors.c_diff_removed()),
        };

        let elapsed = tracker.started.elapsed().as_secs();
        let tag = format!("[{}: {}]", tracker.task_id, tracker.mode);

        let mut spans = vec![
            Span::styled(glyph, Style::default().fg(glyph_color).add_modifier(Modifier::BOLD)),
            Span::styled(tag, Style::default().fg(colors.c_text_primary()).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {elapsed}s · {} tools", tracker.tool_calls), colors.text_dim()),
        ];

        if let Some(ref tool) = tracker.current_tool {
            spans.push(Span::styled(format!(" · {tool}"), Style::default().fg(colors.c_warning())));
        } else if matches!(tracker.status, crate::subagent_tracker::SubagentStatus::Completed { .. }) {
            spans.push(Span::styled(" · done", colors.text_dim()));
        }

        let row_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if row_len > val_w {
            let overflow = row_len.saturating_sub(val_w);
            if let Some(last) = spans.last_mut() {
                let current = last.content.clone();
                let keep = current.chars().count().saturating_sub(overflow + 1);
                let truncated: String = current.chars().take(keep).collect();
                last.content = std::borrow::Cow::Owned(format!("{truncated}…"));
            }
        }

        rows.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(rows), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animate_live_status_rotates_spinner_over_time() {
        let t0 = std::time::Duration::from_millis(0);
        let a = animate_live_status("● assessing…", Some(t0));
        let b = animate_live_status(
            "● assessing…",
            Some(t0 + std::time::Duration::from_millis(160)),
        );
        assert!(
            a.starts_with("⠋ "),
            "first frame should use braille spinner, got {a:?}"
        );
        assert_ne!(a, b, "spinner glyph must move with elapsed time");
        assert!(
            a.ends_with("assessing…"),
            "status body must be preserved, got {a:?}"
        );
        assert!(
            !a.contains('●'),
            "bullet must be replaced by the spinner, got {a:?}"
        );
    }

    #[test]
    fn animate_live_status_keeps_terminal_glyphs_static() {
        assert_eq!(
            animate_live_status("✓ done", Some(std::time::Duration::from_secs(5))),
            "✓ done"
        );
        assert_eq!(
            animate_live_status("✗ Error: boom", Some(std::time::Duration::from_secs(5))),
            "✗ Error: boom"
        );
    }

    #[test]
    fn animate_live_status_defaults_bullet_when_no_elapsed() {
        assert_eq!(animate_live_status("● working…", None), "● working…");
    }
}

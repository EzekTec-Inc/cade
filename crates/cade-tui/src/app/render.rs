use crate::app::layout::question::{question_height, render_question_inline};
use crate::app::layout::pickers::{render_picker, render_theme_picker};
use crate::app::layout::command_palette::render_command_palette;
use crate::app::layout::breadcrumb::render_breadcrumb;
use crate::app::layout::helpers::{mode_sep_color, mode_footer_left, truncate_str};
// Rendering helpers for the TuiApp full-screen layout.
//
// Contains `render_frame` and all supporting free functions for drawing
// the conversation timeline, question panel, picker overlay, footer, etc.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::colors::ThemeColors;
use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

use super::{
    ActiveQuestionDrawState, PlanState, PickerState, RenderLine,
    ThemePickerState, Toast,
    BRAILLE, DOTS, FIXED_ROWS,
    MAX_INPUT_ROWS, SIDEBAR_BREAKPOINT, SIDEBAR_WIDTH,
};
use super::command_palette::CommandPaletteState;
use super::timeline::{
    PreparedTimelineEntry, TimelineEntry, TimelineKey,
    build_timeline_entries, prepare_timeline_entries, render_timeline_viewport,
};
use super::layout::cursor::{calc_input_rows, input_mode_badge};
use super::layout::sidebar::render_sidebar;
use super::layout::toast::render_toast;

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

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_frame(
    frame: &mut Frame,
    lines: &[RenderLine],
    streaming: Option<&str>,
    scroll: usize,
    expand_all: bool,
    textarea: &mut tui_textarea::TextArea<'static>,
    input_mode: InputMode,
    mode: PermissionMode,
    agent_name: &str,
    model: &str,
    last_status: &Option<String>,
    thinking_text: Option<&str>,
    thinking_elapsed: Option<std::time::Duration>,
    active_question: Option<&ActiveQuestionDrawState>,
    pending_lines: usize,
    queued_count: usize,
    cwd: &str,
    context_pct: Option<u8>,
    turn_count: u32,
    token_history: &[u8],
    picker: Option<&PickerState>,
    theme_picker: Option<&ThemePickerState>,
    command_palette: Option<&CommandPaletteState>,
    header_lines: &[RenderLine],
    footer_extra: Option<&str>,
    reasoning_effort: Option<&str>,
    active_plan: Option<&PlanState>,
    copy_mode: bool,
    toast: Option<&Toast>,
    expanded_items: &std::collections::HashSet<TimelineKey>,
    colors: &ThemeColors,
    last_input_width: &mut u16,
    nerd: bool,
) -> u16 {
    // returns max_skip for V-04 scroll clamping
    let area = frame.area();
    let (main_area, sidebar_area) = if area.width >= SIDEBAR_BREAKPOINT {
        let sidebar_w = SIDEBAR_WIDTH.min(area.width.saturating_sub(24));
        let split =
            Layout::horizontal([Constraint::Min(24), Constraint::Length(sidebar_w)]).split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };
    let w = main_area.width as usize;

    let input = textarea.lines().join("\n");
    let (input_badge, _input_badge_color) = input_mode_badge(input_mode, colors);
    let input_prefix_w = input_badge.chars().count() as u16 + 1 + 2;
    let available_w = main_area.width;
    let mut input_rows =
        calc_input_rows(&input, available_w, input_prefix_w).clamp(1, MAX_INPUT_ROWS);

    let inline_h = active_question
        .map(|aq| question_height(aq, main_area.height))
        .unwrap_or(0);

    if inline_h > 0 {
        input_rows = inline_h;
    }

    // A-02: footer_extra adds one row below the normal footer when present.
    let footer_extra_h: u16 = if footer_extra.is_some() {
        1
    } else {
        0
    };
    let bottom_rows = FIXED_ROWS + input_rows + footer_extra_h;

    if main_area.height <= bottom_rows + 1 {
        frame.render_widget(Paragraph::new("Terminal too small"), main_area);
        return 0;
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

    let chunks = if plan_h > 0 {
        Layout::vertical([
            Constraint::Fill(1),                // [0] content  (fluid)
            Constraint::Length(0),              // [1] unused
            Constraint::Length(plan_h),         // [2] plan panel
            Constraint::Length(1),              // [3] status
            Constraint::Length(1),              // [4] top separator
            Constraint::Length(input_rows),     // [5] input or question
            Constraint::Length(1),              // [6] bottom separator
            Constraint::Length(1 + footer_extra_h), // [7] footer
        ])
        .split(main_area)
    } else {
        // No question: same 6-slot layout, pad with two dummy zero-height slots
        // so all index references below are uniform (we only use 0,3..7 in this branch).
        Layout::vertical([
            Constraint::Fill(1),                // [0] content
            Constraint::Length(0),              // [1] (unused)
            Constraint::Length(0),              // [2] (unused)
            Constraint::Length(1),              // [3] status
            Constraint::Length(1),              // [4] top separator
            Constraint::Length(input_rows),     // [5] input or question
            Constraint::Length(1),              // [6] bottom separator
            Constraint::Length(1 + footer_extra_h), // [7] footer
        ])
        .split(main_area)
    };

    // -- A-02: Header strip — pinned above the scrollable messages pane
    let content_w = main_area.width.max(1);
    let (header_area_opt, messages_area) = {
        let mut header_text: Vec<Line<'static>> = Vec::new();
        for entry in build_timeline_entries(header_lines) {
            entry.render_into(w, false, &mut header_text, colors, nerd);
        }
        if header_text.is_empty() {
            (None, chunks[0])
        } else {
            let hh: u16 = header_text
                .iter()
                .map(|l| count_wrapped_rows(l, content_w))
                .sum::<u16>()
                .min(chunks[0].height / 3)
                .max(1);
            let split =
                Layout::vertical([Constraint::Length(hh), Constraint::Min(0)]).split(chunks[0]);
            // Render the pinned header now (before message rendering).
            frame.render_widget(
                Paragraph::new(header_text).wrap(Wrap { trim: false }),
                split[0],
            );
            (Some(split[0]), split[1])
        }
    };
    let _ = header_area_opt; // used above for rendering

    // -- Breadcrumb bar (only on narrow terminals where sidebar is absent)
    let messages_area = if sidebar_area.is_none() && messages_area.height > 4 {
        let [breadcrumb_rect, rest] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(messages_area);
        render_breadcrumb(
            frame,
            breadcrumb_rect,
            model,
            turn_count,
            context_pct,
            token_history,
            colors,
            nerd,
        );
        rest
    } else {
        messages_area
    };

    // -- Content area
    let timeline_w = messages_area.width.saturating_sub(4).max(1) as usize;
    let timeline_entries = build_timeline_entries(lines);
    let mut prepared = prepare_timeline_entries(
        &timeline_entries,
        timeline_w,
        expand_all,
        expanded_items,
        colors,
        nerd,
    );
    if let Some(s) = streaming {
        let next_index = timeline_entries
            .last()
            .map(|e| e.key.index + 1)
            .unwrap_or(0);
        let streaming_entry = TimelineEntry::streaming(next_index, s);
        let mut lines = Vec::new();
        let effective_w = timeline_w.saturating_sub(2);
        streaming_entry.render_with_state(
            effective_w,
            expand_all,
            expanded_items,
            &mut lines,
            colors,
            nerd,
        );
        let rows = lines
            .iter()
            .map(|l| count_wrapped_rows(l, effective_w as u16))
            .sum();
        prepared.push(PreparedTimelineEntry {
            lines,
            rows,
            card_style: crate::app::timeline::CardStyle::Assistant,
        });
    }

    let max_skip = render_timeline_viewport(frame, messages_area, &prepared, scroll, colors);

    // -- A-01: File picker overlay
    if let Some(pk) = picker {
        let n = pk.matches.len().min(6);
        let picker_h = ((2 + n) as u16).clamp(2, messages_area.height.saturating_sub(1));
        let picker_rect = ratatui::layout::Rect {
            x: messages_area.x,
            y: messages_area.y + messages_area.height.saturating_sub(picker_h),
            width: messages_area.width,
            height: picker_h,
        };
        render_picker(frame, pk, picker_rect, colors);
    }

    // -- A-01b: Theme picker overlay
    if let Some(tp) = theme_picker {
        let w = (frame.area().width / 2)
            .max(40)
            .min(frame.area().width.saturating_sub(4));
        let n = tp.filtered_indices.len().max(1).min(10);
        let h = (n as u16 + 4).clamp(5, frame.area().height.saturating_sub(4));

        let r = ratatui::layout::Rect {
            x: frame.area().x + (frame.area().width.saturating_sub(w)) / 2,
            y: frame.area().y + (frame.area().height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        render_theme_picker(frame, tp, r, colors);
    }

    // -- Status row
    let (status_text, status_style) = if let Some(t) = thinking_text {
        let (spinner_text, fg_color) = if let Some(elapsed) = thinking_elapsed {
            let ms = elapsed.as_millis();
            let spinner = if (ms / 3000) % 2 == 0 {
                BRAILLE[(ms / 80) as usize % BRAILLE.len()]
            } else {
                DOTS[(ms / 100) as usize % DOTS.len()]
            };
            let palette: &[(u8, u8, u8)] = &[
                (80, 190, 255),
                (120, 215, 255),
                (160, 235, 255),
                (100, 200, 255),
            ];
            let (r, g, b) = palette[(ms / 400) as usize % palette.len()];
            (
                format!("{} {}", spinner, t),
                ratatui::style::Color::Rgb(r, g, b),
            )
        } else {
            (t.to_string(), colors.accent)
        };
        (
            spinner_text,
            Style::default().fg(fg_color).add_modifier(Modifier::DIM),
        )
    } else if let Some(s) = last_status {
        let fg_color = if s.starts_with('⚠') || s.starts_with('✗') {
            colors.error
        } else {
            colors.success
        };
        (
            s.clone(),
            Style::default().fg(fg_color).add_modifier(Modifier::DIM),
        )
    } else {
        (String::new(), Style::default())
    };

    // Append queued-message badge so the user knows their input was accepted.
    let status_text = if queued_count > 0 {
        format!("{status_text}  · {queued_count} queued")
    } else {
        status_text
    };

    // V-02: Append scroll indicator when user is scrolled up and content is arriving.
    let status_text = if scroll > 0 {
        let hint = if streaming.is_some() {
            "  ↓ streaming…  (Shift+J to follow)".to_string()
        } else if pending_lines > 0 {
            format!("  ↓ {pending_lines} new  (Shift+J to follow)")
        } else {
            String::new()
        };
        if hint.is_empty() {
            status_text
        } else {
            format!("{status_text}{hint}")
        }
    } else {
        status_text
    };

    frame.render_widget(
        Paragraph::new(Span::styled(status_text, status_style)),
        chunks[3],
    );

    // -- Separators
    // U-02: Top separator pulses cyan when the agent is thinking or streaming,
    // giving a peripheral activity signal without cluttering the status bar.
    // Bottom separator always uses the mode color (stable reference point).
    let mode_color = mode_sep_color(mode, colors);
    let top_sep_color = if let Some(elapsed) = thinking_elapsed {
        // Thinking / tool-calling: animated cyan pulse matching the spinner.
        let ms = elapsed.as_millis();
        let palette: &[(u8, u8, u8)] = &[
            (80, 190, 255),
            (120, 215, 255),
            (160, 235, 255),
            (100, 200, 255),
        ];
        let (r, g, b) = palette[(ms / 400) as usize % palette.len()];
        RC::Rgb(r, g, b)
    } else if streaming.is_some() {
        // Pure text streaming (thinking animation already stopped): fixed bright cyan.
        colors.accent
    } else {
        mode_color
    };
    let sep = "─".repeat(main_area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(
            sep.clone(),
            Style::default().fg(top_sep_color),
        )),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(sep, Style::default().fg(mode_color))),
        chunks[6],
    );

    // -- Input area or Question Panel
    if let Some(aq) = active_question {
        render_question_inline(frame, aq, chunks[5], chunks[5], colors);
    } else {
        let (badge_text, badge_color) = input_mode_badge(input_mode, colors);
        let prefix_w = badge_text.chars().count() as u16 + 3;
        
        let input_chunks = Layout::horizontal([
            Constraint::Length(prefix_w),
            Constraint::Fill(1),
        ]).split(chunks[5]);

        let prefix_spans = vec![
            Span::styled(
                badge_text.to_string(),
                Style::default().fg(colors.badge_fg).bg(badge_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled("> ", Style::default().fg(colors.dim)),
        ];
        frame.render_widget(Paragraph::new(Line::from(prefix_spans)), input_chunks[0]);

        let input_placeholder = if queued_count > 0 {
            format!("{queued_count} queued — type another or Ctrl+Enter to redirect")
        } else {
            "Type a message or paste code…".to_string()
        };

        textarea.set_placeholder_text(input_placeholder);
        textarea.set_placeholder_style(Style::default().fg(colors.muted));
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_widget(&*textarea, input_chunks[1]);
        *last_input_width = input_chunks[1].width;
    }

    // -- Footer
    let (left_label, left_glyph, left_color) = mode_footer_left(mode, colors);
    let sidebar_open = sidebar_area.is_some();
    let right_agent = if sidebar_open {
        String::new()
    } else {
        agent_name.to_string()
    };
    let right_model = if sidebar_open {
        String::new()
    } else {
        format!(" [{}]", truncate_str(model, 30))
    };
    let right_reasoning = if sidebar_open {
        String::new()
    } else {
        reasoning_effort
            .map(|r| format!(" [{r}]"))
            .unwrap_or_default()
    };
    let (right_ctx, right_ctx_color) = match context_pct {
        Some(p) if p >= 90 => (format!(" {p}%"), colors.error),
        Some(p) if p >= 80 => (format!(" {p}%"), colors.warning),
        Some(p) => (format!(" {p}%"), colors.muted),
        None => (String::new(), colors.muted),
    };
    let mid_cwd = format!("  {cwd}  ");

    let left_base_len: u16 = left_label.chars().count() as u16
        + if left_glyph.is_empty() {
            0
        } else {
            1 + left_glyph.chars().count() as u16
        };
    let right_len: u16 = (mid_cwd.chars().count()
        + right_agent.chars().count()
        + right_model.chars().count()
        + right_reasoning.chars().count()
        + right_ctx.chars().count()) as u16;
    let pad = chunks[7].width.saturating_sub(left_base_len + right_len) as usize;

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
    footer.push(Span::styled(mid_cwd, Style::default().fg(colors.muted)));
    if !right_agent.is_empty() {
        footer.push(Span::styled(
            right_agent,
            Style::default().fg(colors.thinking_minimal),
        ));
    }
    if !right_model.is_empty() {
        footer.push(Span::styled(right_model, Style::default().fg(colors.dim)));
    }
    if !right_reasoning.is_empty() {
        footer.push(Span::styled(
            right_reasoning,
            Style::default().fg(colors.warning),
        ));
    }
    if !right_ctx.is_empty() {
        footer.push(Span::styled(
            right_ctx,
            Style::default().fg(right_ctx_color),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(footer)), chunks[7]);

    // -- A-02: Footer extra row / selected-block action bar
    if let Some(extra) = footer_extra {
        let extra_rect = ratatui::layout::Rect {
            x: chunks[7].x,
            y: chunks[7].y + 1,
            width: chunks[7].width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_str(extra, extra_rect.width.saturating_sub(1) as usize),
                Style::default().fg(colors.dim),
            )),
            extra_rect,
        );
    }

    if let Some(sidebar) = sidebar_area {
        render_sidebar(
            frame,
            sidebar,
            mode,
            input_mode,
            agent_name,
            model,
            reasoning_effort,
            cwd,
            context_pct,
            turn_count,
            token_history,
            queued_count,
            thinking_text,
            thinking_elapsed,
            active_plan,
            copy_mode,
            colors,
        );
    }

    if let Some(toast) = toast {
        render_toast(frame, main_area, toast, colors);
    }

    // -- Command palette overlay (renders on top of everything)
    if let Some(cp) = command_palette {
        render_command_palette(frame, cp, frame.area(), colors);
    }

    if let Some(plan) = active_plan
        && plan.is_visible
    {
        use ratatui::widgets::{List, ListItem};
        let mut items = Vec::new();
        for step in &plan.steps {
            let (prefix, color) = if step.is_done {
                ("[✓] ", colors.muted)
            } else {
                ("[ ] ", colors.success)
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(color)),
                Span::styled(
                    format!("{}. {}", step.id, step.description),
                    Style::default().fg(if step.is_done {
                        colors.muted
                    } else {
                        colors.text
                    }),
                ),
            ])));
        }
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Todos ")
                .border_style(Style::default().fg(colors.overlay_border)),
        );
        frame.render_widget(list, chunks[2]); // chunks[2] is plan panel in my new chunks array
    }

    max_skip // V-04: returned so draw_impl can clamp self.scroll
}


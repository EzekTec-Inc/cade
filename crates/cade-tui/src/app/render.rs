//! Rendering helpers for the TuiApp full-screen layout.
//!
//! Contains `render_frame` and all supporting free functions for drawing
//! the conversation timeline, question panel, picker overlay, footer, etc.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
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
use super::timeline::{
    PreparedTimelineEntry, TimelineEntry, TimelineKey,
    build_timeline_entries, calc_input_rows, input_mode_badge,
    prepare_timeline_entries, render_sidebar, render_timeline_viewport,
    render_toast,
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
    picker: Option<&PickerState>,
    theme_picker: Option<&ThemePickerState>,
    header_lines: &[RenderLine],
    footer_extra: Option<&str>,
    reasoning_effort: Option<&str>,
    active_plan: Option<&PlanState>,
    copy_mode: bool,
    toast: Option<&Toast>,
    expanded_items: &std::collections::HashSet<TimelineKey>,
    colors: &ThemeColors,
    last_input_width: &mut u16,
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
            entry.render_into(w, false, &mut header_text, colors);
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

    // -- Content area
    let timeline_w = messages_area.width.saturating_sub(4).max(1) as usize;
    let timeline_entries = build_timeline_entries(lines);
    let mut prepared = prepare_timeline_entries(
        &timeline_entries,
        timeline_w,
        expand_all,
        expanded_items,
        colors,
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
                .title(" Todos ")
                .border_style(Style::default().fg(colors.overlay_border)),
        );
        frame.render_widget(list, chunks[2]); // chunks[2] is plan panel in my new chunks array
    }

    max_skip // V-04: returned so draw_impl can clamp self.scroll
}

// -- Overlay helpers

/// Calculate the number of rows needed for the inline question panel.
/// Counts: 1 header + 1 blank + wrapped-question-rows + 1 blank
///       + per-option rows (label + optional description)
///       + submit row (multi-select) + other row + 1 blank + 1 hint.
/// Clamped to at most half the content viewport so content is never fully hidden.
fn question_height(aq: &ActiveQuestionDrawState, content_height: u16) -> u16 {
    let q = &aq.question;

    // Fixed rows: separator-row is accounted for by the caller (inline_h - 1 for body).
    // Here we return the total including the separator row.
    let mut rows: u16 = 0;

    // header chip + blank
    rows += 2;
    // question text (treat as 1 row; long questions word-wrap but we keep it simple)
    rows += 1;
    // blank after question
    rows += 1;

    // progress indicator
    if q.progress.is_some() {
        rows += 2; // "Question N of M" + blank
    }

    // options: label row always, description row only if non-empty
    for idx in 0..aq.total_items {
        if idx == aq.submit_idx {
            rows += 2; // label + blank
        } else if idx == aq.other_idx {
            rows += 2; // label + blank
        } else {
            rows += 1; // label
            if idx < q.options.len() && !q.options[idx].description.is_empty() {
                rows += 1; // description
            }
        }
    }

    // blank + hint
    rows += 2;

    // +1 for the dashed separator row itself
    rows += 1;

    rows.min(content_height / 2).max(6)
}

/// Render the inline question panel — no border box, anchored to the bottom
/// of the content viewport via the layout split in `render_frame`.
/// `sep_area`  — the single row reserved for the dashed separator (chunks[1]).
/// `body_area` — the panel body rows (chunks[2]).
fn render_question_inline(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    sep_area: Rect,
    body_area: Rect,
    colors: &ThemeColors,
) {
    let q = &aq.question;

    // -- Dashed separator
    // Use a dimmer, shorter dash to visually distinguish from the hard ─ separators.
    let dash_w = sep_area.width as usize;
    let dash_str = "╌".repeat(dash_w);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            dash_str,
            Style::default().fg(colors.border),
        ))),
        sep_area,
    );

    // -- Panel body
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header chip — left-aligned, yellow bold with a diamond glyph
    lines.push(Line::from(vec![
        Span::styled("◆ ", Style::default().fg(colors.overlay_section)),
        Span::styled(
            q.header.clone(),
            Style::default()
                .fg(colors.overlay_section)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Question text
    lines.push(Line::from(Span::styled(
        q.text.clone(),
        Style::default().fg(colors.text),
    )));
    lines.push(Line::from(""));

    // Progress indicator
    if let Some((cur, tot)) = q.progress {
        lines.push(Line::from(Span::styled(
            format!("Question {cur} of {tot}"),
            Style::default().fg(colors.muted),
        )));
        lines.push(Line::from(""));
    }

    // Options
    for idx in 0..aq.total_items {
        let is_selected = aq.cursor_pos == idx;
        let selector = if is_selected { "❯" } else { " " };

        // Submit item (multi-select only)
        if idx == aq.submit_idx {
            let style = if is_selected {
                Style::default()
                    .fg(colors.success)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.muted)
            };
            lines.push(Line::from(Span::styled(
                format!(" {selector} {}.  Submit", idx + 1),
                style,
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Free-text "Other" item
        if idx == aq.other_idx {
            let display = if is_selected {
                if aq.custom_text.is_empty() {
                    "Type something.█".to_string()
                } else {
                    format!("{}█", aq.custom_text)
                }
            } else if !aq.custom_text.is_empty() {
                aq.custom_text.clone()
            } else {
                "Type something.".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {selector} {}.  ", idx + 1),
                    Style::default().fg(if is_selected {
                        colors.success
                    } else {
                        colors.muted
                    }),
                ),
                Span::styled(
                    display,
                    Style::default()
                        .fg(colors.dim)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        // Regular option
        let opt = &q.options[idx];
        let checkbox = if q.multi_select {
            if aq.checked[idx] { "[✓] " } else { "[ ] " }
        } else {
            ""
        };
        let num_style = if is_selected {
            Style::default().fg(colors.success)
        } else {
            Style::default().fg(colors.muted)
        };
        let label_style = if is_selected {
            Style::default()
                .fg(colors.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.text)
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {selector} "), Style::default().fg(colors.success)),
            Span::styled(format!("{}. ", idx + 1), num_style),
            Span::styled(checkbox.to_string(), Style::default().fg(colors.success)),
            Span::styled(opt.label.clone(), label_style),
        ]));
        if !opt.description.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("       {}", opt.description),
                Style::default().fg(colors.muted),
            )));
        }
    }

    // Hint line
    lines.push(Line::from(""));
    let hint = if q.multi_select {
        "Enter toggle · ↑↓ navigate · Enter on Submit to confirm · Esc cancel"
    } else {
        "Enter select · ↑↓ navigate · 1-N quick-pick · Esc cancel"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(colors.dim).add_modifier(Modifier::DIM),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body_area);
}

// -- File picker helpers (A-01)

/// Walk `root` up to `max_depth` levels deep, collecting files whose names
/// contain `query` (case-insensitive).  Skips hidden paths and common noise
/// directories (`target`, `node_modules`, `.git`).  Returns relative paths.

/// Render the `@` file picker as a floating overlay at the bottom of `area`.
fn render_picker(frame: &mut Frame, pk: &PickerState, area: Rect, colors: &ThemeColors) {
    if area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Top dashed separator (matches question-panel style)
    lines.push(Line::from(Span::styled(
        "╌".repeat(w),
        Style::default().fg(colors.border),
    )));

    // Header: "@ <query>" + no-match hint
    let no_match = if pk.matches.is_empty() && !pk.query.is_empty() {
        "  (no matches)"
    } else {
        ""
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" @ {}", pk.query),
            Style::default()
                .fg(colors.thinking_minimal)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(no_match, Style::default().fg(colors.muted)),
    ]));

    // Match entries — fill remaining rows (minus sep + header already pushed)
    let max_entries = (area.height as usize).saturating_sub(lines.len());
    for (i, m) in pk.matches.iter().take(max_entries).enumerate() {
        let selected = i == pk.cursor;
        let (glyph, style) = if selected {
            (
                "❯",
                Style::default().fg(colors.text).add_modifier(Modifier::BOLD),
            )
        } else {
            (" ", Style::default().fg(colors.muted))
        };
        lines.push(Line::from(Span::styled(format!(" {glyph} {m}"), style)));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(colors.tool_pending_bg)),
        area,
    );
}

// -- Skills overlay rendering

// -- Path completion (I-02)

/// Try to complete a filesystem path token at `cursor` in `input`.
/// Returns `(new_input, new_cursor)` if a completion was found, `None` otherwise.
/// Only triggers when the token at the cursor starts with `/`, `./`, `~/`, or
/// contains `/` (looks like a path).
// complete_path, collect_files, collect_files_inner, common_prefix
// moved to crate::autocomplete::FileAutocompleteProvider

/// Abbreviate a filesystem path for the footer: last 2 components, with ~/
/// prefix when the path is under the user's home directory.
pub(crate) fn abbreviate_cwd(path: &std::path::Path) -> String {
    let home = dirs::home_dir();
    let (prefix, rel_path) = if let Some(h) = &home {
        if let Ok(rel) = path.strip_prefix(h) {
            ("~/".to_string(), rel.to_path_buf())
        } else {
            (String::new(), path.to_path_buf())
        }
    } else {
        (String::new(), path.to_path_buf())
    };

    let parts: Vec<std::ffi::OsString> = rel_path
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();

    if parts.is_empty() {
        return if prefix.is_empty() {
            "/".to_string()
        } else {
            "~".to_string()
        };
    }

    let display: String = if parts.len() <= 2 {
        parts
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    } else {
        let last2: String = parts[parts.len() - 2..]
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        format!("…/{last2}")
    };

    format!("{prefix}{display}")
}

pub(crate) fn mode_sep_color(mode: PermissionMode, colors: &ThemeColors) -> RC {
    match mode {
        PermissionMode::Default => colors.border_muted,
        PermissionMode::AcceptEdits => colors.thinking_minimal,
        PermissionMode::Plan => colors.success,
        PermissionMode::BypassPermissions => colors.error,
    }
}

fn mode_footer_left<'a>(mode: PermissionMode, colors: &ThemeColors) -> (&'a str, &'a str, RC) {
    match mode {
        PermissionMode::Default => ("Press / for commands", "", colors.border_muted),
        PermissionMode::AcceptEdits => ("accept edits", "⏵⏵", colors.thinking_minimal),
        PermissionMode::Plan => ("plan mode", "⏸", colors.success),
        PermissionMode::BypassPermissions => ("bypass (allow all)", "⚡", colors.error),
    }
}

pub fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Plan => PermissionMode::Default,
        _ => PermissionMode::Plan,
    }
}

pub fn cycle_mode_back(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Plan => PermissionMode::Default,
        _ => PermissionMode::Plan,
    }
}

// -- Misc helpers

pub(crate) fn display_tool_name(name: &str) -> String {
    // Strip MCP server prefix: "developer__shell" → "shell"
    let stripped = if let Some(pos) = name.rfind("__") {
        &name[pos + 2..]
    } else {
        name
    };
    stripped.to_string()
}

/// Produce syntax-highlighted spans for a single line of user input text.
///
/// When the `syntax-highlighting` feature is enabled, this uses syntect with
/// the "base16-ocean.dark" theme to tokenise the line. The syntax is inferred
/// heuristically: if the text looks like it might be code (contains `{`, `(`,
/// `<`, `;`, `fn `, `def `, `import `, etc.) we use a plain-text / generic
/// syntax so tokens still get some colour without false positives.
///

pub fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            chars[..max.saturating_sub(1)].iter().collect::<String>()
        )
    }
}

fn render_theme_picker(
    frame: &mut ratatui::Frame,
    tp: &ThemePickerState,
    area: ratatui::layout::Rect,
    colors: &crate::colors::ThemeColors,
) {
    use ratatui::layout::Constraint;
    use ratatui::style::{Modifier, Style};
    use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

    if area.height == 0 {
        return;
    }

    let hint = " ↑↓ Navigate  Enter Select  Esc/q Cancel ".to_string();
    let rows: Vec<Row> = tp
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(i, &original_idx)| {
            let t = &tp.themes[original_idx];
            let is_sel = i == tp.cursor;

            let style = if is_sel {
                Style::default()
                    .bg(colors.overlay_selected_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(ratatui::text::Span::styled(
                    if is_sel { "▶ " } else { "  " },
                    Style::default().fg(if is_sel {
                        colors.overlay_selected_fg
                    } else {
                        colors.overlay_hint
                    }),
                )),
                Cell::from(ratatui::text::Span::styled(
                    t.name.clone(),
                    Style::default().fg(if is_sel {
                        crate::colors::ThemeColors::dark().text
                    } else {
                        colors.text
                    }),
                )),
                Cell::from(ratatui::text::Span::styled(
                    format!("{:?}", t.source),
                    Style::default().fg(colors.overlay_hint),
                )),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(25),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec!["", "Theme", "Source"]).style(
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Themes {hint}"))
            .border_style(Style::default().fg(colors.overlay_border)),
    );

    let mut ts = ratatui::widgets::TableState::default().with_selected(Some(tp.cursor));

    let main_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)].as_ref())
        .split(area);

    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_stateful_widget(table, main_chunks[0], &mut ts);

    let filter_block = Block::default()
        .borders(Borders::ALL)
        .title(" Filter (Type to search) ")
        .border_style(Style::default().fg(colors.overlay_border));
    let filter_text = Paragraph::new(format!("> {}█", tp.query))
        .block(filter_block)
        .style(Style::default().fg(colors.text));
    frame.render_widget(filter_text, main_chunks[1]);
}

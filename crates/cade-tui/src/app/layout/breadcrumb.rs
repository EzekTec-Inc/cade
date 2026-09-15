//! Breadcrumb/context bar — persistent 1-row strip above the content area.
//! Top-level unified breadcrumb and status bar (Deep module: Viewport Modernization).
//!
//! Replaces separate header, status row, and horizontal separators with an integrated,
//! responsive 1-line breadcrumb and live agent activity bar.

use crate::colors::ThemeColors;
use crate::colors::ThemeColorsExt;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::time::Duration;

const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DOTS: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Context data for the unified top-level breadcrumb and status bar.
#[derive(Debug, Clone)]
pub struct TopBarContext<'a> {
    pub cwd: &'a str,
    pub git_branch: Option<&'a str>,
    pub model: &'a str,
    pub turn_count: u32,
    pub context_pct: Option<u8>,
    pub token_history: &'a [u8],
    pub session_cost_usd: f64,
    pub thinking_text: Option<&'a str>,
    pub thinking_elapsed: Option<Duration>,
    pub last_status: Option<&'a str>,
    pub queued_count: usize,
    pub is_streaming: bool,
    pub scroll: usize,
    pub pending_lines: usize,
    pub nerd: bool,
}

/// Render the unified modern 1-line top bar.
pub(crate) fn render_top_bar(
    frame: &mut Frame,
    area: Rect,
    ctx: &TopBarContext<'_>,
    colors: &ThemeColors,
) {
    if area.width < 24 || area.height == 0 {
        return;
    }

    let sep = Span::styled(" │ ", colors.border_muted());
    let mut left_spans: Vec<Span<'static>> = Vec::new();

    // 1. Brand icon & name
    let brand_icon = if ctx.nerd { "⬢ " } else { "● " };
    left_spans.push(Span::styled(
        format!(" {brand_icon}cade "),
        Style::default()
            .fg(colors.c_primary())
            .add_modifier(Modifier::BOLD),
    ));

    // 2. Working directory & branch
    let branch_info = if let Some(b) = ctx.git_branch {
        format!(" ({b})")
    } else {
        String::new()
    };
    let path_str = format!("› {}{}", ctx.cwd, branch_info);
    let max_path_w = 32usize.min((area.width as usize) / 3);
    let path_display = if path_str.len() > max_path_w && max_path_w > 5 {
        format!("{}…", &path_str[..max_path_w - 1])
    } else {
        path_str
    };
    left_spans.push(Span::styled(path_display, colors.text_dim()));
    left_spans.push(sep.clone());

    // 3. Model name
    let model_icon = if ctx.nerd { "⚡ " } else { "" };
    let max_model_w = 24usize.min((area.width as usize) / 4);
    let model_display = if ctx.model.len() > max_model_w && max_model_w > 4 {
        format!("{}…", &ctx.model[..max_model_w - 1])
    } else {
        ctx.model.to_string()
    };
    left_spans.push(Span::styled(
        format!("{model_icon}{model_display}"),
        colors.text_primary(),
    ));

    // 4. Context usage & Trend
    if let Some(pct) = ctx.context_pct {
        left_spans.push(sep.clone());
        let trend = context_trend(ctx.token_history);
        let trend_icon = match trend {
            Trend::Rising => "↑",
            Trend::Falling => "↓",
            Trend::Stable => "→",
            Trend::Unknown => "·",
        };
        let ctx_color = crate::app::layout::toast::context_severity_color(Some(pct), colors);
        left_spans.push(Span::styled(
            format!("{trend_icon}{pct}%"),
            Style::default().fg(ctx_color),
        ));
    }

    // 5. Cost (if room allows)
    if ctx.session_cost_usd > 0.0 && area.width >= 80 {
        left_spans.push(sep.clone());
        left_spans.push(Span::styled(
            format!("${:.3}", ctx.session_cost_usd),
            colors.text_dim(),
        ));
    }

    // Right-aligned status / hint
    let mut right_spans: Vec<Span<'static>> = Vec::new();
    if let Some(t) = ctx.thinking_text {
        let (spinner, color) = if let Some(elapsed) = ctx.thinking_elapsed {
            let ms = elapsed.as_millis();
            let s = if (ms / 3000) % 2 == 0 {
                BRAILLE[(ms / 80) as usize % BRAILLE.len()]
            } else {
                DOTS[(ms / 100) as usize % DOTS.len()]
            };
            (format!("{} {}", s, t), colors.c_primary())
        } else {
            (format!("● {}", t), colors.c_primary())
        };
        right_spans.push(Span::styled(
            spinner,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    } else if ctx.is_streaming {
        right_spans.push(Span::styled(
            "● Streaming… ",
            Style::default()
                .fg(colors.c_success())
                .add_modifier(Modifier::BOLD),
        ));
    } else if ctx.scroll > 0 {
        let hint = if ctx.pending_lines > 0 {
            format!("↓ {} new (Shift+J) ", ctx.pending_lines)
        } else {
            "↓ Scrolled (Shift+J) ".to_string()
        };
        right_spans.push(Span::styled(
            hint,
            Style::default()
                .fg(colors.c_warning())
                .add_modifier(Modifier::DIM),
        ));
    } else if ctx.queued_count > 0 {
        right_spans.push(Span::styled(
            format!("· {} queued ", ctx.queued_count),
            colors.text_dim(),
        ));
    } else if let Some(s) = ctx.last_status {
        let fg = if s.starts_with('⚠') || s.starts_with('✗') {
            colors.c_error()
        } else {
            colors.c_success()
        };
        right_spans.push(Span::styled(format!("{s} "), Style::default().fg(fg)));
    } else if area.width >= 75 {
        right_spans.push(Span::styled(
            "Ctrl+P commands ",
            Style::default()
                .fg(colors.c_text_muted())
                .add_modifier(Modifier::DIM),
        ));
    }

    // Assemble line with spacing between left and right spans
    let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();
    let right_len: usize = right_spans.iter().map(|s| s.content.chars().count()).sum();
    let total_w = area.width as usize;

    let mut all_spans = left_spans;
    if left_len + right_len < total_w {
        let pad = total_w - left_len - right_len;
        all_spans.push(Span::raw(" ".repeat(pad)));
        all_spans.extend(right_spans);
    }

    frame.render_widget(
        Paragraph::new(Line::from(all_spans)).style(Style::default().bg(colors.c_bg_surface1())),
        area,
    );
}

/// Render the breadcrumb bar.
///
/// Layout: ` Turn 5 │ model-name │ ↑42% ctx │ Ctrl+P palette `
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_breadcrumb(
    frame: &mut Frame,
    area: Rect,
    model: &str,
    turn_count: u32,
    context_pct: Option<u8>,
    token_history: &[u8],
    colors: &ThemeColors,
    nerd: bool,
) {
    if area.width < 20 || area.height == 0 {
        return;
    }

    let sep = Span::styled(" │ ", colors.border_muted());

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Turn indicator
    let turn_icon = if nerd { " " } else { " T" };
    spans.push(Span::styled(
        format!("{}{}", turn_icon, turn_count),
        colors.text_muted(),
    ));

    spans.push(sep.clone());

    // Model name (truncated)
    let max_model_w = 25usize.min(area.width as usize / 3);
    let model_display = if model.len() > max_model_w {
        format!("{}…", &model[..max_model_w - 1])
    } else {
        model.to_string()
    };
    spans.push(Span::styled(model_display, colors.text_dim()));

    spans.push(sep.clone());

    // Context window usage with trend arrow
    if let Some(pct) = context_pct {
        let trend = context_trend(token_history);
        let trend_icon = match trend {
            Trend::Rising => "↑",
            Trend::Falling => "↓",
            Trend::Stable => "→",
            Trend::Unknown => "·",
        };
        let ctx_color = crate::app::layout::toast::context_severity_color(Some(pct), colors);
        spans.push(Span::styled(
            format!("{trend_icon}{pct}% ctx"),
            Style::default().fg(ctx_color),
        ));
    } else {
        spans.push(Span::styled("— ctx", colors.text_dim()));
    }

    // Right-aligned hint (if space allows)
    let used_w: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let hint = " Ctrl+P palette ";
    if used_w + hint.len() + 4 < area.width as usize {
        let pad = area.width as usize - used_w - hint.len();
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            hint.to_string(),
            Style::default()
                .fg(colors.c_text_muted())
                .add_modifier(Modifier::DIM),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(colors.c_bg_surface1())),
        area,
    );
}

// -- Trend calculation

#[derive(Debug, Clone, Copy, PartialEq)]
enum Trend {
    Rising,
    Falling,
    Stable,
    Unknown,
}

/// Determine the recent trend from the last few context-pct samples.
fn context_trend(history: &[u8]) -> Trend {
    if history.len() < 2 {
        return Trend::Unknown;
    }
    // Compare average of last 3 vs previous 3
    let recent = &history[history.len().saturating_sub(3)..];
    let prev_end = history.len().saturating_sub(3);
    let prev_start = prev_end.saturating_sub(3);
    if prev_start >= prev_end {
        return Trend::Unknown;
    }
    let prev = &history[prev_start..prev_end];

    let avg = |slice: &[u8]| -> f32 {
        if slice.is_empty() {
            return 0.0;
        }
        slice.iter().map(|&x| x as f32).sum::<f32>() / slice.len() as f32
    };

    let recent_avg = avg(recent);
    let prev_avg = avg(prev);
    let delta = recent_avg - prev_avg;

    if delta > 3.0 {
        Trend::Rising
    } else if delta < -3.0 {
        Trend::Falling
    } else {
        Trend::Stable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trend_unknown_empty() {
        assert_eq!(context_trend(&[]), Trend::Unknown);
        assert_eq!(context_trend(&[50]), Trend::Unknown);
    }

    #[test]
    fn test_trend_rising() {
        assert_eq!(context_trend(&[10, 15, 20, 30, 40, 50]), Trend::Rising);
    }

    #[test]
    fn test_trend_falling() {
        assert_eq!(context_trend(&[50, 40, 30, 20, 15, 10]), Trend::Falling);
    }

    #[test]
    fn test_trend_stable() {
        assert_eq!(context_trend(&[50, 50, 50, 50, 51, 50]), Trend::Stable);
    }

    #[test]
    fn test_render_top_bar() {
        let backend = ratatui::backend::TestBackend::new(100, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let colors = ThemeColors::default();

        let ctx = TopBarContext {
            cwd: "~/cade",
            git_branch: Some("main"),
            model: "claude-3-7-sonnet",
            turn_count: 5,
            context_pct: Some(42),
            token_history: &[30, 35, 42],
            session_cost_usd: 0.042,
            thinking_text: None,
            thinking_elapsed: None,
            last_status: None,
            queued_count: 0,
            is_streaming: false,
            scroll: 0,
            pending_lines: 0,
            nerd: true,
        };

        terminal
            .draw(|f| {
                render_top_bar(f, f.area(), &ctx, &colors);
            })
            .unwrap();

        // Narrow terminal smoke test: should not panic
        let narrow_backend = ratatui::backend::TestBackend::new(30, 1);
        let mut narrow_term = ratatui::Terminal::new(narrow_backend).unwrap();
        narrow_term
            .draw(|f| {
                render_top_bar(f, f.area(), &ctx, &colors);
            })
            .unwrap();
    }
}

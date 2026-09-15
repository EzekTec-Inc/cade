//! Breadcrumb/status module — the persistent 1-row status strip anchored at the
//! bottom of the viewport (between the content area and the input separator).
//!
//! Holds the breadcrumb-style context (project path, model, context usage, cost)
//! on the left and the live processing animation / activity hints on the right.

use crate::app::layout::helpers::truncate_str;
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

/// Context data for the bottom status bar.
#[derive(Debug, Clone)]
pub struct StatusBarContext<'a> {
    pub cwd: &'a str,
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

/// Render the bottom status bar: breadcrumb context on the left, live
/// processing animation / activity hints on the right.
pub(crate) fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    ctx: &StatusBarContext<'_>,
    colors: &ThemeColors,
) {
    if area.width < 24 || area.height == 0 {
        return;
    }

    let sep = Span::styled(" │ ", colors.border_muted());
    let total_w = area.width as usize;
    let mut left_spans: Vec<Span<'static>> = Vec::new();

    // 1. Project path (abbreviated, char-safe)
    let brand_mark = if ctx.nerd { " ◇ " } else { " › " };
    let path_display = truncate_str(ctx.cwd, (32usize).min(total_w / 3));
    left_spans.push(Span::styled(
        format!("{brand_mark}{path_display}"),
        colors.text_dim(),
    ));
    left_spans.push(sep.clone());

    // 2. Model name (char-safe truncation)
    let max_model_w = 24usize.min(total_w / 3).max(6);
    let model_icon = if ctx.nerd { "◇ " } else { "" };
    let model_display = truncate_str(ctx.model, max_model_w);
    left_spans.push(Span::styled(
        format!("{model_icon}{model_display}"),
        colors.text_primary(),
    ));

    // 3. Turn counter (only once the session has completed turns)
    if ctx.turn_count > 0 {
        left_spans.push(sep.clone());
        left_spans.push(Span::styled(
            format!(
                "{} turn{}",
                ctx.turn_count,
                if ctx.turn_count == 1 { "" } else { "s" }
            ),
            colors.text_dim(),
        ));
    }

    // 4. Context usage & trend
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
            "Ctrl+X leader · Ctrl+P commands ",
            Style::default()
                .fg(colors.c_text_muted())
                .add_modifier(Modifier::DIM),
        ));
    }

    // Assemble line with spacing between left and right spans
    let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();
    let right_len: usize = right_spans.iter().map(|s| s.content.chars().count()).sum();

    let mut all_spans = left_spans;
    if left_len + right_len < total_w {
        let pad = total_w - left_len - right_len;
        all_spans.push(Span::raw(" ".repeat(pad)));
        all_spans.extend(right_spans);
    } else if !right_spans.is_empty() {
        // Very narrow terminal: keep the status but drop the pad. Left spans
        // overflow to the right edge rather than wrapping to a second row.
        all_spans.push(Span::raw("  "));
        all_spans.extend(right_spans);
    }

    frame.render_widget(
        Paragraph::new(Line::from(all_spans)).style(Style::default().bg(colors.c_bg_surface1())),
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
    fn test_trend_unicode_boundary() {
        // Non-ASCII cwd must not panic even when truncated at a charset boundary.
        let ctx = StatusBarContext {
            cwd: "~/Projects/日本語-テスト/とても長いディレクトリ名が入ります",
            model: "anthropic/claude-opus-4.5-2025-12-31",
            turn_count: 3,
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
        let backend = ratatui::backend::TestBackend::new(60, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let colors = ThemeColors::default();
        terminal
            .draw(|f| render_status_bar(f, f.area(), &ctx, &colors))
            .unwrap();
    }

    #[test]
    fn test_render_status_bar() {
        let backend = ratatui::backend::TestBackend::new(100, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let colors = ThemeColors::default();

        let ctx = StatusBarContext {
            cwd: "~/cade",
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
                render_status_bar(f, f.area(), &ctx, &colors);
            })
            .unwrap();

        // Narrow terminal smoke test: should not panic
        let narrow_backend = ratatui::backend::TestBackend::new(30, 1);
        let mut narrow_term = ratatui::Terminal::new(narrow_backend).unwrap();
        narrow_term
            .draw(|f| {
                render_status_bar(f, f.area(), &ctx, &colors);
            })
            .unwrap();
    }
}

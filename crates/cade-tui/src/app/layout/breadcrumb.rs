//! Breadcrumb/context bar — persistent 1-row strip above the content area.
//!
//! Shows turn count, model, context-window usage trend indicator, and
//! key shortcuts — visible on ALL terminal widths (complements the sidebar
//! which only appears on wide terminals ≥ 110 cols).

use crate::colors::ThemeColors;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Render the breadcrumb bar.
///
/// Layout: ` Turn 5 │ model-name │ ↑42% ctx │ Ctrl+P palette `
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

    let sep = Span::styled(" │ ", Style::default().fg(colors.border_muted));

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Turn indicator
    let turn_icon = if nerd { " " } else { " T" };
    spans.push(Span::styled(
        format!("{}{}",turn_icon, turn_count),
        Style::default().fg(colors.muted),
    ));

    spans.push(sep.clone());

    // Model name (truncated)
    let max_model_w = 25usize.min(area.width as usize / 3);
    let model_display = if model.len() > max_model_w {
        format!("{}…", &model[..max_model_w - 1])
    } else {
        model.to_string()
    };
    spans.push(Span::styled(
        model_display,
        Style::default().fg(colors.dim),
    ));

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
        spans.push(Span::styled(
            "— ctx",
            Style::default().fg(colors.dim),
        ));
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
                .fg(colors.overlay_hint)
                .add_modifier(Modifier::DIM),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(colors.reasoning_bg)),
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
}

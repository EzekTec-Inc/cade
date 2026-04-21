use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::session::PlanState;

/// Render the plan panel inside the sidebar when a plan is active.
///
/// Mirrors the TUI plan panel: a checklist of steps with [✓]/[ ] prefixes,
/// done steps are dimmed, active steps are bright.
pub fn render(ui: &mut egui::Ui, plan: &PlanState, theme: &crate::theme::ThemeColors) {
    if !plan.is_visible || plan.steps.is_empty() {
        return;
    }

    ui.add_space(4.0);

    // Thin top separator
    let sep_rect = ui.available_rect_before_wrap();
    let sep_rect = egui::Rect::from_min_size(sep_rect.min, egui::vec2(sep_rect.width(), 1.0));
    ui.painter().rect_filled(sep_rect, 0.0, theme.border_base());
    ui.advance_cursor_after_rect(sep_rect);

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(" Todos ")
            .color(theme.text_muted())
            .monospace()
            .strong()
            .size(11.0),
    );
    ui.add_space(2.0);

    for step in &plan.steps {
        let (prefix, prefix_color) = if step.is_done {
            ("[✓]", theme.text_muted())
        } else {
            ("[ ]", theme.success())
        };

        let desc_color = if step.is_done {
            theme.text_muted()
        } else {
            theme.text_primary()
        };

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(prefix)
                    .color(prefix_color)
                    .monospace()
                    .size(11.0),
            );
            ui.label(
                egui::RichText::new(format!("{}. {}", step.id, step.description))
                    .color(desc_color)
                    .monospace()
                    .size(11.0),
            );
        });
    }

    ui.add_space(2.0);

    // Thin bottom separator
    let sep_rect = ui.available_rect_before_wrap();
    let sep_rect = egui::Rect::from_min_size(sep_rect.min, egui::vec2(sep_rect.width(), 1.0));
    ui.painter().rect_filled(sep_rect, 0.0, theme.border_base());
    ui.advance_cursor_after_rect(sep_rect);
}

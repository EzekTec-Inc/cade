use eframe::egui;
use egui_plot::{Bar, BarChart, Plot};

pub fn render(ui: &mut egui::Ui, theme: &crate::theme::ThemeColors) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(20.0);
        
        ui.heading(egui::RichText::new("Overview").color(theme.text_primary()));
        ui.add_space(20.0);

        ui.columns(2, |columns| {
            // Left Column
            columns[0].vertical(|ui| {
                // What's New Section
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("What's New").color(theme.primary()));
                    ui.label("CADE has been updated to support hybrid tab-based dashboard views.");
                });
                
                ui.add_space(20.0);

                // Current Configuration
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Current Configuration").color(theme.primary()));
                    ui.label("Model: Auto-detect");
                    ui.label("Permissions: Default");
                    ui.label("Memory: Advanced enabled");
                });
            });

            // Right Column
            columns[1].vertical(|ui| {
                // Tool Usage Chart
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Tool Usage").color(theme.primary()));
                    let chart = BarChart::new(vec![
                        Bar::new(0.5, 10.0).name("bash").fill(theme.primary()),
                        Bar::new(1.5, 5.0).name("read_file").fill(theme.success()),
                        Bar::new(2.5, 8.0).name("edit_file").fill(theme.warning()),
                        Bar::new(3.5, 2.0).name("update_memory").fill(theme.error()),
                    ])
                    .width(0.8)
                    .name("Usage Count");

                    Plot::new("tool_usage_plot")
                        .view_aspect(2.0)
                        .show(ui, |plot_ui| plot_ui.bar_chart(chart));
                });

                ui.add_space(20.0);

                // Execution Queue
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Executions Queue").color(theme.primary()));
                    ui.label("No active background executions.");
                });
                
                ui.add_space(20.0);
                
                // Last Execution
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Last Execution").color(theme.primary()));
                    ui.label("Status: Success");
                    ui.label("Duration: 1.2s");
                });
            });
        });
    });
}

import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/sidebar.rs', 'r') as f:
    sidebar = f.read()

sidebar = sidebar.replace("""pub fn render(
    ui: &mut egui::Ui,
    _session: &crate::session::ConnectedSession,
    _theme: &crate::theme::ThemeColors,
) {
""", """pub fn render(
    ui: &mut egui::Ui,
    active_page: &mut crate::app::ActivePage,
    _session: &Option<crate::session::SessionState>,
    _theme: &crate::theme::ThemeColors,
) -> Option<crate::app::AppAction> {
    let mut action = None;
""")

sidebar = sidebar.replace("""    egui::CentralPanel::default().frame(frame).show_inside(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
""", """    egui::Panel::left("dashboard_sidebar")
        .frame(frame)
        .exact_size(240.0)
        .resizable(false)
        .show_inside(ui, |ui| {
""")

# Wait, the sidebar.rs I currently have IS the new sidebar. Let's check what it looks like.

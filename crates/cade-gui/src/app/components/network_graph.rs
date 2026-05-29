use crate::session::{ConnectedSession, NetworkGraphTopology, NetworkNodeKind};
use crate::theme::EguiThemeExt;
use eframe::egui;

pub fn render(ui: &mut egui::Ui, session: &ConnectedSession, theme: &crate::theme::ThemeColors) {
    let graph = session.network_graph_topology();
    let frame = egui::Frame::NONE
        .fill(theme.bg_card())
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(18))
        .stroke(egui::Stroke::new(1.0, theme.border_base()));

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Network Node Graph")
                    .color(theme.text_primary())
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} nodes · {} edges",
                    graph.nodes.len(),
                    graph.edges.len()
                ))
                .color(theme.text_muted())
                .size(11.0)
                .monospace(),
            );
        });
        ui.add_space(14.0);

        let desired = egui::vec2(ui.available_width(), 300.0);
        let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        draw_graph(&painter, rect, &graph, theme);
        response.on_hover_text(
            "Live topology: agent, model, context, memory, MCP servers, and callable tools",
        );
    });
}

fn draw_graph(
    painter: &egui::Painter,
    rect: egui::Rect,
    graph: &NetworkGraphTopology,
    theme: &crate::theme::ThemeColors,
) {
    painter.rect_filled(rect, egui::CornerRadius::same(6), theme.bg_surface0());
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        egui::Stroke::new(1.0, theme.border_base()),
        egui::StrokeKind::Inside,
    );

    if graph.nodes.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No active topology yet",
            egui::FontId::proportional(14.0),
            theme.text_muted(),
        );
        return;
    }

    let mut positions = std::collections::BTreeMap::new();
    let lanes = [
        NetworkNodeKind::Memory,
        NetworkNodeKind::McpServer,
        NetworkNodeKind::Agent,
        NetworkNodeKind::Model,
        NetworkNodeKind::Context,
        NetworkNodeKind::Tool,
    ];

    for (lane_idx, kind) in lanes.iter().enumerate() {
        let nodes: Vec<_> = graph
            .nodes
            .iter()
            .filter(|node| node.kind == *kind)
            .collect();
        if nodes.is_empty() {
            continue;
        }
        let x = rect.left() + 52.0 + lane_idx as f32 * ((rect.width() - 104.0) / 5.0).max(1.0);
        for (idx, node) in nodes.iter().enumerate() {
            let y = rect.top()
                + 42.0
                + idx as f32 * ((rect.height() - 84.0) / nodes.len().max(1) as f32);
            positions.insert(node.id.as_str(), egui::pos2(x, y));
        }
    }

    for edge in &graph.edges {
        if let (Some(from), Some(to)) = (
            positions.get(edge.from.as_str()),
            positions.get(edge.to.as_str()),
        ) {
            painter.line_segment([*from, *to], egui::Stroke::new(1.0, theme.text_dim()));
            let mid = egui::pos2((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
            painter.circle_filled(mid, 2.0, theme.primary());
        }
    }

    for node in &graph.nodes {
        if let Some(pos) = positions.get(node.id.as_str()) {
            let color = node_color(node.kind, theme);
            let radius = if node.kind == NetworkNodeKind::Agent {
                28.0
            } else {
                22.0
            };
            painter.circle_filled(*pos, radius, theme.tinted_bg(color, 48));
            painter.circle_stroke(*pos, radius, egui::Stroke::new(2.0, color));
            painter.text(
                *pos,
                egui::Align2::CENTER_CENTER,
                node_icon(node.kind),
                egui::FontId::proportional(18.0),
                color,
            );
            painter.text(
                egui::pos2(pos.x, pos.y + radius + 12.0),
                egui::Align2::CENTER_CENTER,
                compact_label(&node.label, 18),
                egui::FontId::proportional(11.0),
                theme.text_primary(),
            );
            painter.text(
                egui::pos2(pos.x, pos.y + radius + 25.0),
                egui::Align2::CENTER_CENTER,
                compact_label(&node.meta, 20),
                egui::FontId::monospace(9.0),
                theme.text_muted(),
            );
        }
    }
}

fn node_color(kind: NetworkNodeKind, theme: &crate::theme::ThemeColors) -> egui::Color32 {
    match kind {
        NetworkNodeKind::Agent => theme.primary(),
        NetworkNodeKind::Model => theme.purple(),
        NetworkNodeKind::Tool => theme.warning(),
        NetworkNodeKind::Memory => theme.success(),
        NetworkNodeKind::McpServer => theme.teal(),
        NetworkNodeKind::Context => theme.accent_dim(),
    }
}

fn node_icon(kind: NetworkNodeKind) -> &'static str {
    match kind {
        NetworkNodeKind::Agent => "A",
        NetworkNodeKind::Model => "M",
        NetworkNodeKind::Tool => "T",
        NetworkNodeKind::Memory => "μ",
        NetworkNodeKind::McpServer => "S",
        NetworkNodeKind::Context => "C",
    }
}

fn compact_label(label: &str, max_chars: usize) -> String {
    let mut out: String = label.chars().take(max_chars).collect();
    if label.chars().count() > max_chars {
        out.push('…');
    }
    out
}

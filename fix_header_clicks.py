import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/header.rs', 'r') as f:
    header = f.read()

header = header.replace('ui.add(btn);', 'if ui.add(btn).clicked() { }')
header = header.replace('ui.add(egui::Label::new(egui::RichText::new("Manage LLM keys").color(link_color).size(13.0)).sense(egui::Sense::click()));', 'if ui.add(egui::Label::new(egui::RichText::new("Manage LLM keys").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }')
header = header.replace('ui.add(egui::Label::new(egui::RichText::new("API reference").color(link_color).size(13.0)).sense(egui::Sense::click()));', 'if ui.add(egui::Label::new(egui::RichText::new("API reference").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }')
header = header.replace('ui.add(egui::Label::new(egui::RichText::new("Docs").color(link_color).size(13.0)).sense(egui::Sense::click()));', 'if ui.add(egui::Label::new(egui::RichText::new("Docs").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }')
header = header.replace('ui.add(egui::Label::new(egui::RichText::new("Support").color(link_color).size(13.0)).sense(egui::Sense::click()));', 'if ui.add(egui::Label::new(egui::RichText::new("Support").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }')

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/header.rs', 'w') as f:
    f.write(header)


use dioxus::prelude::*;
use crate::types::SelectedPage;

/// Color accents for visual grouping of features and tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileAccent {
    Sky,
    Emerald,
    Amber,
    Purple,
}

impl TileAccent {
    pub fn border_hover(&self) -> &'static str {
        match self {
            Self::Sky => "hover:border-sky-500/60 focus-visible:border-sky-400",
            Self::Emerald => "hover:border-emerald-500/60 focus-visible:border-emerald-400",
            Self::Amber => "hover:border-amber-500/60 focus-visible:border-amber-400",
            Self::Purple => "hover:border-purple-500/60 focus-visible:border-purple-400",
        }
    }

    pub fn bg_gradient(&self) -> &'static str {
        match self {
            Self::Sky => "from-blue-950/40 via-slate-900/60 to-[#090d16]",
            Self::Emerald => "from-emerald-950/40 via-slate-900/60 to-[#090d16]",
            Self::Amber => "from-amber-950/40 via-slate-900/60 to-[#090d16]",
            Self::Purple => "from-purple-950/40 via-slate-900/60 to-[#090d16]",
        }
    }

    pub fn text_accent(&self) -> &'static str {
        match self {
            Self::Sky => "text-sky-400",
            Self::Emerald => "text-emerald-400",
            Self::Amber => "text-amber-400",
            Self::Purple => "text-purple-400",
        }
    }

    pub fn badge_classes(&self) -> &'static str {
        match self {
            Self::Sky => "bg-sky-950/90 text-sky-400 border-sky-800/80",
            Self::Emerald => "bg-emerald-950/90 text-emerald-400 border-emerald-800/80",
            Self::Amber => "bg-amber-950/90 text-amber-400 border-amber-800/80",
            Self::Purple => "bg-purple-950/90 text-purple-400 border-purple-800/80",
        }
    }
}

/// Semantic iconography options for FeatureTile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileIcon {
    Desktop,
    Terminal,
    Sdk,
    Workflow,
    Shield,
}

#[component]
fn RenderTileIcon(icon: TileIcon, accent: TileAccent) -> Element {
    let color_cls = accent.text_accent();
    match icon {
        TileIcon::Desktop => rsx! {
            svg { class: "w-20 h-20 {color_cls} filter drop-shadow-[0_0_12px_rgba(14,165,233,0.15)]", view_box: "0 0 100 100",
                circle { cx: "50", cy: "50", r: "32", fill: "none", stroke: "currentColor", "stroke-width": "1.5" }
                circle { cx: "50", cy: "50", r: "20", fill: "currentColor", "fill-opacity": "0.2" }
                rect { x: "42", y: "42", width: "16", height: "16", rx: "3", fill: "currentColor" }
            }
        },
        TileIcon::Terminal => rsx! {
            svg { class: "w-20 h-20 {color_cls} filter drop-shadow-[0_0_12px_rgba(16,185,129,0.15)]", view_box: "0 0 100 100",
                rect { x: "25", y: "30", width: "50", height: "40", rx: "6", fill: "#0f172a", stroke: "currentColor", "stroke-width": "1.5" }
                text { x: "32", y: "52", fill: "currentColor", "font-family": "monospace", "font-size": "14", "font-weight": "bold", ">_ " }
            }
        },
        TileIcon::Sdk => rsx! {
            svg { class: "w-20 h-20 {color_cls} filter drop-shadow-[0_0_12px_rgba(245,158,11,0.15)]", view_box: "0 0 100 100",
                rect { x: "30", y: "30", width: "40", height: "40", rx: "6", fill: "#0f172a", stroke: "currentColor", "stroke-width": "1.5" }
                circle { cx: "50", cy: "50", r: "8", fill: "currentColor" }
            }
        },
        TileIcon::Workflow => rsx! {
            svg { class: "w-20 h-20 {color_cls}", view_box: "0 0 100 100",
                circle { cx: "30", cy: "50", r: "10", stroke: "currentColor", "stroke-width": "2", fill: "none" }
                circle { cx: "70", cy: "50", r: "10", stroke: "currentColor", "stroke-width": "2", fill: "none" }
                path { d: "M 40 50 L 60 50", stroke: "currentColor", "stroke-width": "2" }
            }
        },
        TileIcon::Shield => rsx! {
            svg { class: "w-20 h-20 {color_cls}", view_box: "0 0 100 100",
                path { d: "M 50 20 L 75 30 L 75 55 C 75 70 50 82 50 82 C 50 82 25 70 25 55 L 25 30 Z", stroke: "currentColor", "stroke-width": "2", fill: "none" }
            }
        },
    }
}

/// Deep reusable feature card encapsulating accessibility, hover styling,
/// badge positioning, icon rendering, and single-click navigation.
#[component]
pub fn FeatureTile(
    title: &'static str,
    description: &'static str,
    badge: &'static str,
    action_label: &'static str,
    icon: TileIcon,
    accent: TileAccent,
    destination: SelectedPage,
    mut active_page: Signal<SelectedPage>,
) -> Element {
    let hover_cls = accent.border_hover();
    let bg_grad = accent.bg_gradient();
    let text_acc = accent.text_accent();
    let badge_cls = accent.badge_classes();

    rsx! {
        div {
            class: "bg-[#090d16] border border-slate-800/90 rounded-xl overflow-hidden {hover_cls} group transition-all duration-200 flex flex-col justify-between shadow-md cursor-pointer select-none focus-visible:ring-2 focus-visible:ring-sky-400 outline-none",
            tabindex: 0,
            role: "button",
            "aria-label": "{title}: {description}",
            onclick: move |_| active_page.set(destination),
            onkeydown: move |evt: KeyboardEvent| {
                if evt.key() == Key::Enter || evt.key() == Key::Character(" ".to_string()) {
                    active_page.set(destination);
                }
            },
            div { class: "relative h-36 bg-gradient-to-br {bg_grad} flex items-center justify-center p-4 overflow-hidden border-b border-slate-800/60",
                RenderTileIcon { icon, accent }
                span { class: "absolute top-3 right-3 text-[10px] font-mono font-bold px-2 py-0.5 rounded-full border uppercase tracking-wider {badge_cls}", "{badge}" }
            }
            div { class: "p-5 flex-1 flex flex-col justify-between",
                div {
                    h3 { class: "text-slate-100 font-bold text-sm mb-1.5 group-hover:{text_acc} transition-colors duration-150", "{title}" }
                    p { class: "text-slate-400 text-xs leading-relaxed text-pretty", "{description}" }
                }
                span { class: "text-[11px] {text_acc} group-hover:translate-x-1 transition-transform inline-flex items-center space-x-1 font-medium mt-3.5",
                    span { "{action_label}" }
                    span { "→" }
                }
            }
        }
    }
}

/// Standardized high-density metric card.
#[component]
pub fn MetricTile(
    label: String,
    value: String,
    subtext: Option<String>,
    accent: Option<TileAccent>,
) -> Element {
    let border_accent = accent.map(|a| a.text_accent()).unwrap_or("text-slate-400");

    rsx! {
        div { class: "bg-slate-900/60 border border-slate-800/80 rounded-xl p-4 flex flex-col justify-between shadow-sm select-none",
            span { class: "text-xs text-slate-400 font-medium tracking-tight mb-1", "{label}" }
            div { class: "text-2xl font-bold font-mono tabular-nums text-slate-100 my-0.5", "{value}" }
            if let Some(sub) = subtext {
                span { class: "text-[11px] {border_accent} font-medium mt-1", "{sub}" }
            }
        }
    }
}

/// Interactive tab pill with accessibility attributes and focus ring.
#[component]
pub fn TabPill(
    active: bool,
    label: String,
    onclick: EventHandler<()>,
) -> Element {
    let cls = if active {
        "px-3.5 py-1.5 bg-slate-800 text-sky-400 rounded-lg cursor-pointer border border-slate-700 font-medium text-xs shadow-sm transition-all focus-visible:ring-2 focus-visible:ring-sky-400 outline-none select-none"
    } else {
        "px-3.5 py-1.5 text-slate-400 hover:text-slate-200 hover:bg-slate-800/40 rounded-lg cursor-pointer text-xs transition-colors duration-150 border border-transparent focus-visible:ring-2 focus-visible:ring-sky-400 outline-none select-none"
    };

    rsx! {
        button {
            class: "{cls}",
            role: "tab",
            "aria-selected": "{active}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_accent_classes() {
        assert!(TileAccent::Sky.text_accent().contains("sky"));
        assert!(TileAccent::Emerald.text_accent().contains("emerald"));
        assert!(TileAccent::Amber.text_accent().contains("amber"));
        assert!(TileAccent::Purple.text_accent().contains("purple"));
    }

    #[test]
    fn test_tile_accent_hover_borders() {
        assert!(TileAccent::Sky.border_hover().contains("hover:border-sky"));
        assert!(TileAccent::Emerald.border_hover().contains("hover:border-emerald"));
    }
}

//! Browser projection of the same semantic tokens used by the terminal.
pub fn style(name: Option<&str>) -> String {
    let theme = name.and_then(|name| {
        cade_core::resources::get_theme(name).or_else(|| {
            cade_core::resources::list_available_themes()
                .into_iter()
                .find(|theme| {
                    theme.name.eq_ignore_ascii_case(name)
                        || theme.display_name.eq_ignore_ascii_case(name)
                })
                .and_then(|theme| cade_core::resources::get_theme(&theme.name))
        })
    });
    let roles = [
        ("--cade-bg", "bg.base", "#0f1115"),
        ("--cade-panel", "bg.panel", "#16171d"),
        ("--cade-elevated", "bg.elevated", "#20232c"),
        ("--cade-text", "text.primary", "#e5e7eb"),
        ("--cade-muted", "text.muted", "#9ca3af"),
        ("--cade-border", "border.unfocused", "#272833"),
        ("--cade-accent", "accent.primary", "#ff7c5c"),
        ("--cade-error", "error", "#f87171"),
    ];
    let mut style: String = roles
        .iter()
        .map(|(css, role, fallback)| {
            let color = theme
                .as_ref()
                .and_then(|t| cade_core::resources::resolve_token(t, role, &[]).rgb())
                .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"))
                .unwrap_or_else(|| (*fallback).to_owned());
            format!("{css}:{color};")
        })
        .collect();
    let accent = theme
        .as_ref()
        .and_then(|t| cade_core::resources::resolve_token(t, "accent.primary", &[]).rgb())
        .unwrap_or((255, 124, 92));
    let light = cade_core::resources::calculate_contrast_ratio(accent, (255, 255, 255));
    let dark = cade_core::resources::calculate_contrast_ratio(accent, (0, 0, 0));
    style.push_str(if light > dark {
        "--cade-accent-ink:#ffffff;"
    } else {
        "--cade-accent-ink:#000000;"
    });
    style
}

pub const INTERACTION_CSS: &str = r#"
.cade-prompt-bar { margin: 0 1rem .75rem; padding: .8rem 1rem; border: 1px solid var(--cade-border); border-radius: .85rem; background: var(--cade-panel); color: var(--cade-text); display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
.cade-prompt-bar > span, .cade-dialog header > div { min-width: 0; overflow-wrap: anywhere; }
.cade-dialog { width: min(42rem, calc(100vw - 2rem)); max-height: calc(100dvh - 2rem); padding: 0; margin: auto; border: 1px solid var(--cade-border); border-radius: 1rem; background: var(--cade-panel); color: var(--cade-text); box-shadow: 0 24px 80px #0008; overflow-y: auto; }
.cade-dialog::backdrop { background: #0009; backdrop-filter: blur(4px); }
.cade-dialog header, .cade-dialog footer { padding: 1.25rem 1.5rem; display: flex; gap: .75rem; align-items: center; justify-content: space-between; }
.cade-dialog header { border-bottom: 1px solid var(--cade-border); }
.cade-dialog footer { border-top: 1px solid var(--cade-border); flex-wrap: wrap; }
.cade-dialog h2 { font-size: 1.15rem; font-weight: 650; overflow-wrap: anywhere; }
.cade-dialog .prompt-body { padding: 1.5rem; display: grid; gap: 1.25rem; }
.cade-dialog .prompt-muted { color: var(--cade-muted); font-size: .85rem; overflow-wrap: anywhere; }
.cade-dialog fieldset { display: grid; gap: .65rem; min-width: 0; }
.cade-dialog legend { font-weight: 600; margin-bottom: .75rem; overflow-wrap: anywhere; }
.cade-dialog .prompt-body p { white-space: pre-wrap; overflow-wrap: anywhere; }
.cade-dialog .prompt-choice { display: flex; align-items: flex-start; gap: .75rem; border: 1px solid var(--cade-border); border-radius: .75rem; padding: .9rem; cursor: pointer; background: var(--cade-elevated); }
.cade-dialog .prompt-choice:has(input:checked) { border-color: var(--cade-accent); }
.cade-dialog .prompt-choice input { margin-top: .25rem; accent-color: var(--cade-accent); flex-shrink: 0; }
.cade-dialog .prompt-choice strong { display: block; font-weight: 600; }
.cade-dialog .prompt-choice span { min-width: 0; overflow-wrap: anywhere; }
.cade-dialog textarea, .cade-dialog pre { width: 100%; border: 1px solid var(--cade-border); background: var(--cade-bg); color: var(--cade-text); border-radius: .65rem; padding: .85rem; font-size: .875rem; }
.cade-dialog textarea { min-height: 5rem; resize: vertical; }
.cade-dialog pre { max-height: 16rem; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
.cade-prompt-button { border: 1px solid var(--cade-border); background: var(--cade-elevated); color: var(--cade-text); border-radius: .6rem; padding: .6rem 1rem; font-weight: 600; font-size: .875rem; cursor: pointer; }
.cade-prompt-button.primary { background: var(--cade-accent); color: var(--cade-accent-ink); border-color: var(--cade-accent); }
.cade-prompt-button:disabled { opacity: .5; cursor: wait; }
.cade-prompt-button:focus-visible, .cade-dialog textarea:focus-visible, .cade-dialog input:focus-visible { outline: 2px solid var(--cade-accent); outline-offset: 3px; }
.cade-dialog .prompt-error { color: var(--cade-error); font-size: .875rem; }
"#;

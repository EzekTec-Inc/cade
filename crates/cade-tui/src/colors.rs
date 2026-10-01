pub use cade_core::resources::Theme as ThemeColors;

/// Include the resolved palette, not just the theme name: custom themes can be
/// reloaded in place. Cache users own invalidation even outside TuiApp.
pub(crate) fn theme_fingerprint(colors: &ThemeColors) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", colors.meta).hash(&mut hash);
    let mut tokens = colors.token_names();
    tokens.extend(colors.palette_names());
    tokens.sort_unstable();
    tokens.dedup();
    for token in tokens {
        token.hash(&mut hash);
        format!("{:?}", colors.color(token)).hash(&mut hash);
    }
    let mut styles = colors.style_names();
    styles.sort_unstable();
    for name in styles {
        name.hash(&mut hash);
        format!("{:?}", colors.style(name)).hash(&mut hash);
    }
    hash.finish()
}
use ratatui::style::{Color as RC, Modifier, Style}; // Alias to Opaline Theme

pub trait ThemeColorsExt {
    fn style_base(&self) -> Style;
    fn style_surface0(&self) -> Style;
    fn style_surface1(&self) -> Style;
    fn style_surface2(&self) -> Style;

    fn text_primary(&self) -> Style;
    fn text_muted(&self) -> Style;
    fn text_dim(&self) -> Style;

    fn text_primary_bold(&self) -> Style;
    fn text_muted_bold(&self) -> Style;

    fn border_base(&self) -> Style;
    fn border_focus(&self) -> Style;
    fn border_muted(&self) -> Style;
    fn border_accent(&self) -> Style;

    fn primary(&self) -> Style;
    fn primary_bold(&self) -> Style;
    fn success(&self) -> Style;
    fn error(&self) -> Style;
    fn warning(&self) -> Style;

    fn badge(&self) -> Style;

    fn diff_added(&self) -> Style;
    fn diff_removed(&self) -> Style;
    fn diff_context(&self) -> Style;

    fn md_heading(&self) -> Style;
    fn md_link(&self) -> Style;
    fn md_link_url(&self) -> Style;
    fn md_code(&self) -> Style;
    fn md_code_block(&self) -> Style;
    fn md_code_block_border(&self) -> Style;
    fn md_quote(&self) -> Style;
    fn md_quote_border(&self) -> Style;
    fn md_hr(&self) -> Style;
    fn md_list_bullet(&self) -> Style;

    fn syntax_comment(&self) -> Style;
    fn syntax_keyword(&self) -> Style;
    fn syntax_function(&self) -> Style;
    fn syntax_variable(&self) -> Style;
    fn syntax_string(&self) -> Style;
    fn syntax_number(&self) -> Style;
    fn syntax_type(&self) -> Style;
    fn syntax_operator(&self) -> Style;
    fn syntax_punctuation(&self) -> Style;

    fn thinking_off(&self) -> Style;
    fn thinking_minimal(&self) -> Style;
    fn thinking_low(&self) -> Style;
    fn thinking_medium(&self) -> Style;
    fn thinking_high(&self) -> Style;
    fn thinking_xhigh(&self) -> Style;

    fn bash_mode(&self) -> Style;
    fn bg_card_style(&self) -> Style;
    fn selected_bg_style(&self) -> Style;
    fn tool_success_bg_style(&self) -> Style;
    fn tool_error_bg_style(&self) -> Style;
    fn tool_pending_bg_style(&self) -> Style;

    // Direct color accessors (replacing old struct fields)
    fn c_bg_base(&self) -> RC;
    fn c_bg_surface0(&self) -> RC;
    fn c_bg_surface1(&self) -> RC;
    fn c_bg_surface2(&self) -> RC;
    fn c_primary(&self) -> RC;
    fn c_success(&self) -> RC;
    fn c_error(&self) -> RC;
    fn c_warning(&self) -> RC;
    fn c_text_primary(&self) -> RC;
    fn c_text_muted(&self) -> RC;
    fn c_text_dim(&self) -> RC;
    fn c_border_base(&self) -> RC;
    fn c_border_focus(&self) -> RC;
    fn c_border_muted(&self) -> RC;
    fn c_border_accent(&self) -> RC;

    // Extended tokens
    fn c_diff_added(&self) -> RC;
    fn c_diff_removed(&self) -> RC;
    fn c_diff_context(&self) -> RC;
    fn c_md_heading(&self) -> RC;
    fn c_md_link(&self) -> RC;
    fn c_md_link_url(&self) -> RC;
    fn c_md_code(&self) -> RC;
    fn c_md_code_block(&self) -> RC;
    fn c_md_code_block_border(&self) -> RC;
    fn c_md_quote(&self) -> RC;
    fn c_md_quote_border(&self) -> RC;
    fn c_md_hr(&self) -> RC;
    fn c_md_list_bullet(&self) -> RC;

    fn c_syntax_comment(&self) -> RC;
    fn c_syntax_keyword(&self) -> RC;
    fn c_syntax_function(&self) -> RC;
    fn c_syntax_variable(&self) -> RC;
    fn c_syntax_string(&self) -> RC;
    fn c_syntax_number(&self) -> RC;
    fn c_syntax_type(&self) -> RC;
    fn c_syntax_operator(&self) -> RC;
    fn c_syntax_punctuation(&self) -> RC;

    fn c_thinking_off(&self) -> RC;
    fn c_thinking_minimal(&self) -> RC;
    fn c_thinking_low(&self) -> RC;
    fn c_thinking_medium(&self) -> RC;
    fn c_thinking_high(&self) -> RC;
    fn c_thinking_xhigh(&self) -> RC;

    fn c_bash_mode(&self) -> RC;
    fn c_bg_card(&self) -> RC;
    fn c_bg_input(&self) -> RC;
    fn c_selected_bg(&self) -> RC;
    fn c_tool_success_bg(&self) -> RC;
    fn c_tool_error_bg(&self) -> RC;
    fn c_tool_pending_bg(&self) -> RC;

    fn c_ctx_bar_system(&self) -> RC;
    fn c_ctx_bar_native_tools(&self) -> RC;
    fn c_ctx_bar_mcp_tools(&self) -> RC;
    fn c_ctx_bar_memory(&self) -> RC;
    fn c_ctx_bar_skills(&self) -> RC;
    fn c_ctx_bar_messages(&self) -> RC;
    fn c_ctx_bar_free(&self) -> RC;
    fn c_ctx_bar_buffer(&self) -> RC;
    fn c_spinner_0(&self) -> RC;
    fn c_spinner_1(&self) -> RC;
    fn c_spinner_2(&self) -> RC;
    fn c_spinner_3(&self) -> RC;

    fn c_border_style(&self) -> ratatui::widgets::BorderType;
}

fn resolve_fallback(theme: &ThemeColors, primary: &str, fallback: &str) -> RC {
    if let Some(c) = theme.try_color(primary) {
        c.into()
    } else if let Some(c) = theme.try_color(fallback) {
        c.into()
    } else {
        // Missing tokens inherit a semantic base, never Opaline's sentinel
        // color for both foreground and background. Reset preserves the
        // terminal's own defaults when a sparse theme defines no base token.
        let base = if primary.starts_with("bg.") {
            "bg.base"
        } else {
            "text.primary"
        };
        theme.try_color(base).map(Into::into).unwrap_or(RC::Reset)
    }
}

impl ThemeColorsExt for ThemeColors {
    fn c_bg_base(&self) -> RC {
        self.try_color("bg.base")
            .map(Into::into)
            .unwrap_or(RC::Reset)
    }
    fn c_bg_surface0(&self) -> RC {
        resolve_fallback(self, "bg.panel", "cade.user_message_bg")
    }
    fn c_bg_surface1(&self) -> RC {
        resolve_fallback(self, "bg.elevated", "cade.tool_success_bg")
    }
    fn c_bg_surface2(&self) -> RC {
        resolve_fallback(self, "bg.highlight", "cade.selected_bg")
    }

    fn c_primary(&self) -> RC {
        resolve_fallback(self, "accent.primary", "text.primary")
    }
    fn c_success(&self) -> RC {
        resolve_fallback(self, "success", "cade.success")
    }
    fn c_error(&self) -> RC {
        resolve_fallback(self, "error", "cade.error")
    }
    fn c_warning(&self) -> RC {
        resolve_fallback(self, "warning", "cade.warning")
    }

    fn c_text_primary(&self) -> RC {
        self.try_color("text.primary")
            .map(Into::into)
            .unwrap_or(RC::Reset)
    }
    fn c_text_muted(&self) -> RC {
        resolve_fallback(self, "text.muted", "text.primary")
    }
    fn c_text_dim(&self) -> RC {
        resolve_fallback(self, "text.dim", "text.muted")
    }

    fn c_border_base(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_border_focus(&self) -> RC {
        resolve_fallback(self, "border.focused", "cade.border_accent")
    }
    fn c_border_muted(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_border_accent(&self) -> RC {
        resolve_fallback(self, "border.focused", "cade.border_accent")
    }

    fn c_diff_added(&self) -> RC {
        resolve_fallback(self, "success", "cade.success")
    }
    fn c_diff_removed(&self) -> RC {
        resolve_fallback(self, "error", "cade.error")
    }
    fn c_diff_context(&self) -> RC {
        self.c_text_muted()
    }

    fn c_md_heading(&self) -> RC {
        resolve_fallback(self, "warning", "cade.warning")
    }
    fn c_md_link(&self) -> RC {
        self.c_primary()
    }
    fn c_md_link_url(&self) -> RC {
        self.c_text_muted()
    }
    fn c_md_code(&self) -> RC {
        resolve_fallback(self, "accent.secondary", "accent.primary")
    }
    fn c_md_code_block(&self) -> RC {
        self.c_text_primary()
    }
    fn c_md_code_block_border(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_md_quote(&self) -> RC {
        self.c_text_muted()
    }
    fn c_md_quote_border(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_md_hr(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_md_list_bullet(&self) -> RC {
        self.c_primary()
    }

    fn c_syntax_comment(&self) -> RC {
        resolve_fallback(self, "code.comment", "cade.syntax_comment")
    }
    fn c_syntax_keyword(&self) -> RC {
        resolve_fallback(self, "code.keyword", "cade.syntax_keyword")
    }
    fn c_syntax_function(&self) -> RC {
        resolve_fallback(self, "code.function", "cade.syntax_function")
    }
    fn c_syntax_variable(&self) -> RC {
        self.c_text_primary()
    }
    fn c_syntax_string(&self) -> RC {
        resolve_fallback(self, "code.string", "cade.syntax_string")
    }
    fn c_syntax_number(&self) -> RC {
        resolve_fallback(self, "code.number", "cade.syntax_number")
    }
    fn c_syntax_type(&self) -> RC {
        resolve_fallback(self, "code.type", "cade.syntax_type")
    }
    fn c_syntax_operator(&self) -> RC {
        resolve_fallback(self, "code.keyword", "cade.syntax_keyword")
    }
    fn c_syntax_punctuation(&self) -> RC {
        self.c_text_muted()
    }

    fn c_thinking_off(&self) -> RC {
        self.c_text_dim()
    }
    fn c_thinking_minimal(&self) -> RC {
        self.c_primary()
    }
    fn c_thinking_low(&self) -> RC {
        resolve_fallback(self, "accent.secondary", "accent.primary")
    }
    fn c_thinking_medium(&self) -> RC {
        resolve_fallback(self, "success", "cade.success")
    }
    fn c_thinking_high(&self) -> RC {
        resolve_fallback(self, "warning", "cade.warning")
    }
    fn c_thinking_xhigh(&self) -> RC {
        resolve_fallback(self, "error", "cade.error")
    }

    fn c_bash_mode(&self) -> RC {
        resolve_fallback(self, "warning", "cade.warning")
    }
    fn c_bg_card(&self) -> RC {
        resolve_fallback(self, "bg.elevated", "cade.tool_success_bg")
    }
    fn c_bg_input(&self) -> RC {
        resolve_fallback(self, "bg.panel", "cade.user_message_bg")
    }
    fn c_selected_bg(&self) -> RC {
        resolve_fallback(self, "bg.selection", "cade.selected_bg")
    }
    fn c_tool_success_bg(&self) -> RC {
        resolve_fallback(self, "bg.elevated", "cade.tool_success_bg")
    }
    fn c_tool_error_bg(&self) -> RC {
        resolve_fallback(self, "bg.highlight", "cade.selected_bg")
    }
    fn c_tool_pending_bg(&self) -> RC {
        resolve_fallback(self, "bg.panel", "cade.user_message_bg")
    }

    fn c_ctx_bar_system(&self) -> RC {
        self.c_text_dim()
    }
    fn c_ctx_bar_native_tools(&self) -> RC {
        resolve_fallback(self, "accent.secondary", "accent.primary")
    }
    fn c_ctx_bar_mcp_tools(&self) -> RC {
        resolve_fallback(self, "accent.tertiary", "accent.primary")
    }
    fn c_ctx_bar_memory(&self) -> RC {
        resolve_fallback(self, "warning", "cade.warning")
    }
    fn c_ctx_bar_skills(&self) -> RC {
        self.c_primary()
    }
    fn c_ctx_bar_messages(&self) -> RC {
        resolve_fallback(self, "accent.deep", "accent.primary")
    }
    fn c_ctx_bar_free(&self) -> RC {
        self.c_text_dim()
    }
    fn c_ctx_bar_buffer(&self) -> RC {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn c_spinner_0(&self) -> RC {
        self.c_primary()
    }
    fn c_spinner_1(&self) -> RC {
        self.c_primary()
    }
    fn c_spinner_2(&self) -> RC {
        self.c_primary()
    }
    fn c_spinner_3(&self) -> RC {
        self.c_primary()
    }

    fn c_border_style(&self) -> ratatui::widgets::BorderType {
        ratatui::widgets::BorderType::Rounded
    }

    fn style_base(&self) -> Style {
        Style::default()
            .bg(self.c_bg_base())
            .fg(self.c_text_primary())
    }
    fn style_surface0(&self) -> Style {
        Style::default()
            .bg(self.c_bg_surface0())
            .fg(self.c_text_primary())
    }
    fn style_surface1(&self) -> Style {
        Style::default()
            .bg(self.c_bg_surface1())
            .fg(self.c_text_primary())
    }
    fn style_surface2(&self) -> Style {
        Style::default()
            .bg(self.c_bg_surface2())
            .fg(self.c_text_primary())
    }

    fn text_primary(&self) -> Style {
        Style::default().fg(self.c_text_primary())
    }
    fn text_muted(&self) -> Style {
        Style::default().fg(self.c_text_muted())
    }
    fn text_dim(&self) -> Style {
        Style::default().fg(self.c_text_dim())
    }

    fn text_primary_bold(&self) -> Style {
        Style::default()
            .fg(self.c_text_primary())
            .add_modifier(Modifier::BOLD)
    }
    fn text_muted_bold(&self) -> Style {
        Style::default()
            .fg(self.c_text_muted())
            .add_modifier(Modifier::BOLD)
    }

    fn border_base(&self) -> Style {
        Style::default().fg(self.c_border_base())
    }
    fn border_focus(&self) -> Style {
        Style::default().fg(self.c_border_focus())
    }
    fn border_muted(&self) -> Style {
        Style::default().fg(self.c_border_muted())
    }
    fn border_accent(&self) -> Style {
        Style::default().fg(self.c_border_accent())
    }

    fn primary(&self) -> Style {
        Style::default().fg(self.c_primary())
    }
    fn primary_bold(&self) -> Style {
        Style::default()
            .fg(self.c_primary())
            .add_modifier(Modifier::BOLD)
    }
    fn success(&self) -> Style {
        Style::default().fg(self.c_success())
    }
    fn error(&self) -> Style {
        Style::default().fg(self.c_error())
    }
    fn warning(&self) -> Style {
        Style::default().fg(self.c_warning())
    }

    fn badge(&self) -> Style {
        Style::default()
            .bg(self.c_bg_surface2())
            .fg(self.c_primary())
    }

    fn diff_added(&self) -> Style {
        Style::default().fg(self.c_diff_added())
    }
    fn diff_removed(&self) -> Style {
        Style::default().fg(self.c_diff_removed())
    }
    fn diff_context(&self) -> Style {
        Style::default().fg(self.c_diff_context())
    }

    fn md_heading(&self) -> Style {
        Style::default().fg(self.c_md_heading())
    }
    fn md_link(&self) -> Style {
        Style::default().fg(self.c_md_link())
    }
    fn md_link_url(&self) -> Style {
        Style::default().fg(self.c_md_link_url())
    }
    fn md_code(&self) -> Style {
        Style::default().fg(self.c_md_code())
    }
    fn md_code_block(&self) -> Style {
        Style::default().fg(self.c_md_code_block())
    }
    fn md_code_block_border(&self) -> Style {
        Style::default().fg(self.c_md_code_block_border())
    }
    fn md_quote(&self) -> Style {
        Style::default().fg(self.c_md_quote())
    }
    fn md_quote_border(&self) -> Style {
        Style::default().fg(self.c_md_quote_border())
    }
    fn md_hr(&self) -> Style {
        Style::default().fg(self.c_md_hr())
    }
    fn md_list_bullet(&self) -> Style {
        Style::default().fg(self.c_md_list_bullet())
    }

    fn syntax_comment(&self) -> Style {
        Style::default().fg(self.c_syntax_comment())
    }
    fn syntax_keyword(&self) -> Style {
        Style::default().fg(self.c_syntax_keyword())
    }
    fn syntax_function(&self) -> Style {
        Style::default().fg(self.c_syntax_function())
    }
    fn syntax_variable(&self) -> Style {
        Style::default().fg(self.c_syntax_variable())
    }
    fn syntax_string(&self) -> Style {
        Style::default().fg(self.c_syntax_string())
    }
    fn syntax_number(&self) -> Style {
        Style::default().fg(self.c_syntax_number())
    }
    fn syntax_type(&self) -> Style {
        Style::default().fg(self.c_syntax_type())
    }
    fn syntax_operator(&self) -> Style {
        Style::default().fg(self.c_syntax_operator())
    }
    fn syntax_punctuation(&self) -> Style {
        Style::default().fg(self.c_syntax_punctuation())
    }

    fn thinking_off(&self) -> Style {
        Style::default().fg(self.c_thinking_off())
    }
    fn thinking_minimal(&self) -> Style {
        Style::default().fg(self.c_thinking_minimal())
    }
    fn thinking_low(&self) -> Style {
        Style::default().fg(self.c_thinking_low())
    }
    fn thinking_medium(&self) -> Style {
        Style::default().fg(self.c_thinking_medium())
    }
    fn thinking_high(&self) -> Style {
        Style::default().fg(self.c_thinking_high())
    }
    fn thinking_xhigh(&self) -> Style {
        Style::default().fg(self.c_thinking_xhigh())
    }

    fn bash_mode(&self) -> Style {
        Style::default().fg(self.c_bash_mode())
    }
    fn bg_card_style(&self) -> Style {
        Style::default()
            .bg(self.c_bg_card())
            .fg(self.c_text_primary())
    }
    fn selected_bg_style(&self) -> Style {
        Style::default()
            .bg(self.c_selected_bg())
            .fg(self.c_text_primary())
    }
    fn tool_success_bg_style(&self) -> Style {
        Style::default()
            .bg(self.c_tool_success_bg())
            .fg(self.c_text_primary())
    }
    fn tool_error_bg_style(&self) -> Style {
        Style::default()
            .bg(self.c_tool_error_bg())
            .fg(self.c_text_primary())
    }
    fn tool_pending_bg_style(&self) -> Style {
        Style::default()
            .bg(self.c_tool_pending_bg())
            .fg(self.c_text_primary())
    }
}

#[cfg(feature = "syntax-highlighting")]
pub fn generate_syntect_theme(colors: &ThemeColors) -> syntect::highlighting::Theme {
    use syntect::highlighting::{Color, StyleModifier, ThemeItem, ThemeSettings};
    let rgb = |color| match color {
        RC::Rgb(r, g, b) => Some(Color { r, g, b, a: 255 }),
        _ => None,
    };
    let foreground = rgb(colors.c_text_primary());
    let background = rgb(colors.c_bg_surface0());
    let scopes = [
        ("comment", colors.c_syntax_comment()),
        ("keyword, storage", colors.c_syntax_keyword()),
        ("string", colors.c_syntax_string()),
        ("constant.numeric", colors.c_syntax_number()),
        ("entity.name.function", colors.c_syntax_function()),
        ("entity.name.type", colors.c_syntax_type()),
    ]
    .into_iter()
    .filter_map(|(scope, color)| {
        Some(ThemeItem {
            scope: scope.parse().ok()?,
            style: StyleModifier {
                foreground: rgb(color),
                ..Default::default()
            },
        })
    })
    .collect();
    syntect::highlighting::Theme {
        name: Some("CadeDynamic".to_string()),
        author: Some("CADE".to_string()),
        settings: ThemeSettings {
            foreground,
            background,
            caret: foreground,
            line_highlight: rgb(colors.c_bg_surface2()),
            misspelling: rgb(colors.c_error()),
            minimap_border: None,
            accent: None,
            popup_css: None,
            phantom_css: None,
            bracket_contents_foreground: None,
            bracket_contents_options: None,
            brackets_foreground: None,
            brackets_background: None,
            brackets_options: None,
            tags_foreground: None,
            tags_options: None,
            find_highlight: None,
            find_highlight_foreground: None,
            gutter: None,
            gutter_foreground: None,
            selection: None,
            selection_foreground: None,
            selection_border: None,
            inactive_selection: None,
            inactive_selection_foreground: None,
            guide: None,
            active_guide: None,
            stack_guide: None,
            highlight: None,
            shadow: None,
        },
        scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neutral_gray_preserved_in_fallback_resolution() {
        let toml = r##"
        [meta]
        name = "neutral-gray-tui"
        variant = "dark"
        [palette]
        gray = "#808080"
        accent = "#112233"
        [tokens]
        "bg.panel" = "gray"
        "cade.user_message_bg" = "accent"
        "##;
        let theme = opaline::load_from_str(toml, None).expect("valid toml");
        // c_bg_surface0 attempts "bg.panel" first with fallback "cade.user_message_bg"
        let surface0 = theme.c_bg_surface0();
        assert_eq!(surface0, RC::Rgb(128, 128, 128));
    }

    #[test]
    fn test_fallback_selected_when_primary_token_absent() {
        let toml = r##"
        [meta]
        name = "fallback-tui"
        variant = "dark"
        [palette]
        accent = "#112233"
        [tokens]
        "cade.user_message_bg" = "accent"
        "##;
        let theme = opaline::load_from_str(toml, None).expect("valid toml");
        let surface0 = theme.c_bg_surface0();
        assert_eq!(surface0, RC::Rgb(0x11, 0x22, 0x33));
    }
}

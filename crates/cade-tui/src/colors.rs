use ratatui::style::{Color as RC, Style, Modifier};
pub use cade_core::resources::Theme as ThemeColors; // Alias to Opaline Theme

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

impl ThemeColorsExt for ThemeColors {
    fn c_bg_base(&self) -> RC { self.color("bg.base").into() }
    fn c_bg_surface0(&self) -> RC { self.color("bg.panel").into() }
    fn c_bg_surface1(&self) -> RC { self.color("bg.elevated").into() }
    fn c_bg_surface2(&self) -> RC { self.color("bg.highlight").into() }
    
    fn c_primary(&self) -> RC { self.color("accent.primary").into() }
    fn c_success(&self) -> RC { self.color("success").into() }
    fn c_error(&self) -> RC { self.color("error").into() }
    fn c_warning(&self) -> RC { self.color("warning").into() }
    
    fn c_text_primary(&self) -> RC { self.color("text.primary").into() }
    fn c_text_muted(&self) -> RC { self.color("text.muted").into() }
    fn c_text_dim(&self) -> RC { self.color("text.dim").into() }
    
    fn c_border_base(&self) -> RC { self.color("border.unfocused").into() }
    fn c_border_focus(&self) -> RC { self.color("border.focused").into() }
    fn c_border_muted(&self) -> RC { self.color("border.unfocused").into() }
    fn c_border_accent(&self) -> RC { self.color("border.focused").into() }
    
    fn c_diff_added(&self) -> RC { self.color("cade.tool_diff_added").into() }
    fn c_diff_removed(&self) -> RC { self.color("cade.tool_diff_removed").into() }
    fn c_diff_context(&self) -> RC { self.color("cade.tool_diff_context").into() }
    
    fn c_md_heading(&self) -> RC { self.color("cade.md_heading").into() }
    fn c_md_link(&self) -> RC { self.color("cade.md_link").into() }
    fn c_md_link_url(&self) -> RC { self.color("cade.md_link_url").into() }
    fn c_md_code(&self) -> RC { self.color("cade.md_code").into() }
    fn c_md_code_block(&self) -> RC { self.color("cade.md_code_block").into() }
    fn c_md_code_block_border(&self) -> RC { self.color("cade.md_code_block_border").into() }
    fn c_md_quote(&self) -> RC { self.color("cade.md_quote").into() }
    fn c_md_quote_border(&self) -> RC { self.color("cade.md_quote_border").into() }
    fn c_md_hr(&self) -> RC { self.color("cade.md_hr").into() }
    fn c_md_list_bullet(&self) -> RC { self.color("cade.md_list_bullet").into() }
    
    fn c_syntax_comment(&self) -> RC { self.color("cade.syntax_comment").into() }
    fn c_syntax_keyword(&self) -> RC { self.color("cade.syntax_keyword").into() }
    fn c_syntax_function(&self) -> RC { self.color("cade.syntax_function").into() }
    fn c_syntax_variable(&self) -> RC { self.color("cade.syntax_variable").into() }
    fn c_syntax_string(&self) -> RC { self.color("cade.syntax_string").into() }
    fn c_syntax_number(&self) -> RC { self.color("cade.syntax_number").into() }
    fn c_syntax_type(&self) -> RC { self.color("cade.syntax_type").into() }
    fn c_syntax_operator(&self) -> RC { self.color("cade.syntax_operator").into() }
    fn c_syntax_punctuation(&self) -> RC { self.color("cade.syntax_punctuation").into() }
    
    fn c_thinking_off(&self) -> RC { self.color("cade.thinking_off").into() }
    fn c_thinking_minimal(&self) -> RC { self.color("cade.thinking_minimal").into() }
    fn c_thinking_low(&self) -> RC { self.color("cade.thinking_low").into() }
    fn c_thinking_medium(&self) -> RC { self.color("cade.thinking_medium").into() }
    fn c_thinking_high(&self) -> RC { self.color("cade.thinking_high").into() }
    fn c_thinking_xhigh(&self) -> RC { self.color("cade.thinking_xhigh").into() }
    
    fn c_bash_mode(&self) -> RC { self.color("cade.bash_mode").into() }
    fn c_bg_card(&self) -> RC { self.color("cade.bg_card").into() }
    fn c_bg_input(&self) -> RC { self.color("bg.surface").into() }
    fn c_selected_bg(&self) -> RC { self.color("bg.selection").into() }
    fn c_tool_success_bg(&self) -> RC { self.color("cade.tool_success_bg").into() }
    fn c_tool_error_bg(&self) -> RC { self.color("cade.tool_error_bg").into() }
    fn c_tool_pending_bg(&self) -> RC { self.color("cade.tool_pending_bg").into() }

    fn c_ctx_bar_system(&self) -> RC { self.color("cade.ctx_bar_system").into() }
    fn c_ctx_bar_native_tools(&self) -> RC { self.color("cade.ctx_bar_native_tools").into() }
    fn c_ctx_bar_mcp_tools(&self) -> RC { self.color("cade.ctx_bar_mcp_tools").into() }
    fn c_ctx_bar_memory(&self) -> RC { self.color("cade.ctx_bar_memory").into() }
    fn c_ctx_bar_skills(&self) -> RC { self.color("cade.ctx_bar_skills").into() }
    fn c_ctx_bar_messages(&self) -> RC { self.color("cade.ctx_bar_messages").into() }
    fn c_ctx_bar_free(&self) -> RC { self.color("cade.ctx_bar_free").into() }
    fn c_ctx_bar_buffer(&self) -> RC { self.color("cade.ctx_bar_buffer").into() }
    fn c_spinner_0(&self) -> RC { self.color("accent.primary").into() }
    fn c_spinner_1(&self) -> RC { self.color("accent.primary").into() }
    fn c_spinner_2(&self) -> RC { self.color("accent.primary").into() }
    fn c_spinner_3(&self) -> RC { self.color("accent.primary").into() }

    fn c_border_style(&self) -> ratatui::widgets::BorderType { ratatui::widgets::BorderType::Rounded }


    fn style_base(&self) -> Style { Style::default().bg(self.c_bg_base()).fg(self.c_text_primary()) }
    fn style_surface0(&self) -> Style { Style::default().bg(self.c_bg_surface0()).fg(self.c_text_primary()) }
    fn style_surface1(&self) -> Style { Style::default().bg(self.c_bg_surface1()).fg(self.c_text_primary()) }
    fn style_surface2(&self) -> Style { Style::default().bg(self.c_bg_surface2()).fg(self.c_text_primary()) }

    fn text_primary(&self) -> Style { Style::default().fg(self.c_text_primary()) }
    fn text_muted(&self) -> Style { Style::default().fg(self.c_text_muted()) }
    fn text_dim(&self) -> Style { Style::default().fg(self.c_text_dim()) }
    
    fn text_primary_bold(&self) -> Style { Style::default().fg(self.c_text_primary()).add_modifier(Modifier::BOLD) }
    fn text_muted_bold(&self) -> Style { Style::default().fg(self.c_text_muted()).add_modifier(Modifier::BOLD) }

    fn border_base(&self) -> Style { Style::default().fg(self.c_border_base()) }
    fn border_focus(&self) -> Style { Style::default().fg(self.c_border_focus()) }
    fn border_muted(&self) -> Style { Style::default().fg(self.c_border_muted()) }
    fn border_accent(&self) -> Style { Style::default().fg(self.c_border_accent()) }
    
    fn primary(&self) -> Style { Style::default().fg(self.c_primary()) }
    fn primary_bold(&self) -> Style { Style::default().fg(self.c_primary()).add_modifier(Modifier::BOLD) }
    fn success(&self) -> Style { Style::default().fg(self.c_success()) }
    fn error(&self) -> Style { Style::default().fg(self.c_error()) }
    fn warning(&self) -> Style { Style::default().fg(self.c_warning()) }

    fn badge(&self) -> Style { Style::default().bg(self.c_bg_surface2()).fg(self.c_primary()) }

    fn diff_added(&self) -> Style { Style::default().fg(self.c_diff_added()) }
    fn diff_removed(&self) -> Style { Style::default().fg(self.c_diff_removed()) }
    fn diff_context(&self) -> Style { Style::default().fg(self.c_diff_context()) }

    fn md_heading(&self) -> Style { Style::default().fg(self.c_md_heading()) }
    fn md_link(&self) -> Style { Style::default().fg(self.c_md_link()) }
    fn md_link_url(&self) -> Style { Style::default().fg(self.c_md_link_url()) }
    fn md_code(&self) -> Style { Style::default().fg(self.c_md_code()) }
    fn md_code_block(&self) -> Style { Style::default().fg(self.c_md_code_block()) }
    fn md_code_block_border(&self) -> Style { Style::default().fg(self.c_md_code_block_border()) }
    fn md_quote(&self) -> Style { Style::default().fg(self.c_md_quote()) }
    fn md_quote_border(&self) -> Style { Style::default().fg(self.c_md_quote_border()) }
    fn md_hr(&self) -> Style { Style::default().fg(self.c_md_hr()) }
    fn md_list_bullet(&self) -> Style { Style::default().fg(self.c_md_list_bullet()) }

    fn syntax_comment(&self) -> Style { Style::default().fg(self.c_syntax_comment()) }
    fn syntax_keyword(&self) -> Style { Style::default().fg(self.c_syntax_keyword()) }
    fn syntax_function(&self) -> Style { Style::default().fg(self.c_syntax_function()) }
    fn syntax_variable(&self) -> Style { Style::default().fg(self.c_syntax_variable()) }
    fn syntax_string(&self) -> Style { Style::default().fg(self.c_syntax_string()) }
    fn syntax_number(&self) -> Style { Style::default().fg(self.c_syntax_number()) }
    fn syntax_type(&self) -> Style { Style::default().fg(self.c_syntax_type()) }
    fn syntax_operator(&self) -> Style { Style::default().fg(self.c_syntax_operator()) }
    fn syntax_punctuation(&self) -> Style { Style::default().fg(self.c_syntax_punctuation()) }

    fn thinking_off(&self) -> Style { Style::default().fg(self.c_thinking_off()) }
    fn thinking_minimal(&self) -> Style { Style::default().fg(self.c_thinking_minimal()) }
    fn thinking_low(&self) -> Style { Style::default().fg(self.c_thinking_low()) }
    fn thinking_medium(&self) -> Style { Style::default().fg(self.c_thinking_medium()) }
    fn thinking_high(&self) -> Style { Style::default().fg(self.c_thinking_high()) }
    fn thinking_xhigh(&self) -> Style { Style::default().fg(self.c_thinking_xhigh()) }

    fn bash_mode(&self) -> Style { Style::default().fg(self.c_bash_mode()) }
    fn bg_card_style(&self) -> Style { Style::default().bg(self.c_bg_card()).fg(self.c_text_primary()) }
    fn selected_bg_style(&self) -> Style { Style::default().bg(self.c_selected_bg()).fg(self.c_text_primary()) }
    fn tool_success_bg_style(&self) -> Style { Style::default().bg(self.c_tool_success_bg()).fg(self.c_text_primary()) }
    fn tool_error_bg_style(&self) -> Style { Style::default().bg(self.c_tool_error_bg()).fg(self.c_text_primary()) }
    fn tool_pending_bg_style(&self) -> Style { Style::default().bg(self.c_tool_pending_bg()).fg(self.c_text_primary()) }
}

#[cfg(feature = "syntax-highlighting")]
pub fn generate_syntect_theme(colors: &ThemeColors) -> syntect::highlighting::Theme {
    use syntect::highlighting::{Color, ThemeItem, ThemeSettings, ScopeSelectors};
    let mut theme = syntect::highlighting::Theme {
        name: Some("CadeDynamic".to_string()),
        author: Some("CADE".to_string()),
        settings: ThemeSettings {
            foreground: Some(Color { r: 205, g: 214, b: 244, a: 255 }),
            background: Some(Color { r: 30, g: 30, b: 46, a: 255 }),
            caret: Some(Color { r: 205, g: 214, b: 244, a: 255 }),
            line_highlight: Some(Color { r: 30, g: 30, b: 46, a: 255 }),
            misspelling: Some(Color { r: 243, g: 139, b: 168, a: 255 }),
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
        scopes: Vec::new(),
    };
    theme
}
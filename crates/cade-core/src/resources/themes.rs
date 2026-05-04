/// Theme discovery and loading.
///
/// Themes are JSON files in `~/.cade/themes/*.json` or `.cade/themes/*.json`.
/// The `name` key inside the JSON is the theme identifier used in settings.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// region:    --- Types

/// A single color value in a theme.
///
/// Supported formats (matching pi's theme schema):
/// - `"#rrggbb"` — 24-bit hex
/// - An integer 0–255 — xterm 256-color index
/// - A string referencing a variable defined in `vars`
/// - `""` — terminal default color
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ThemeColor {
    Hex(String), // "#rrggbb" or variable name or ""
    Index(u8),   // 0-255 palette index
}

impl Default for ThemeColor {
    fn default() -> Self {
        Self::Hex(String::new())
    }
}

/// Full set of color tokens for a theme (51 tokens matching pi's schema).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThemeTokens {
    // -- Core UI
    pub accent: ThemeColor,
    pub border: ThemeColor,
    #[serde(rename = "borderAccent")]
    pub border_accent: ThemeColor,
    #[serde(rename = "borderMuted")]
    pub border_muted: ThemeColor,
    pub success: ThemeColor,
    pub error: ThemeColor,
    pub warning: ThemeColor,
    pub muted: ThemeColor,
    pub dim: ThemeColor,
    pub text: ThemeColor,
    #[serde(rename = "thinkingText")]
    pub thinking_text: ThemeColor,

    // -- Backgrounds & content
    #[serde(rename = "selectedBg")]
    pub selected_bg: ThemeColor,
    #[serde(rename = "userMessageBg")]
    pub user_message_bg: ThemeColor,
    #[serde(rename = "userMessageText")]
    pub user_message_text: ThemeColor,
    #[serde(rename = "customMessageBg")]
    pub custom_message_bg: ThemeColor,
    #[serde(rename = "customMessageText")]
    pub custom_message_text: ThemeColor,
    #[serde(rename = "customMessageLabel")]
    pub custom_message_label: ThemeColor,
    #[serde(rename = "toolPendingBg")]
    pub tool_pending_bg: ThemeColor,
    #[serde(rename = "toolSuccessBg")]
    pub tool_success_bg: ThemeColor,
    #[serde(rename = "toolErrorBg")]
    pub tool_error_bg: ThemeColor,
    #[serde(rename = "toolTitle")]
    pub tool_title: ThemeColor,
    #[serde(rename = "toolOutput")]
    pub tool_output: ThemeColor,

    // -- Markdown
    #[serde(rename = "mdHeading")]
    pub md_heading: ThemeColor,
    #[serde(rename = "mdLink")]
    pub md_link: ThemeColor,
    #[serde(rename = "mdLinkUrl")]
    pub md_link_url: ThemeColor,
    #[serde(rename = "mdCode")]
    pub md_code: ThemeColor,
    #[serde(rename = "mdCodeBlock")]
    pub md_code_block: ThemeColor,
    #[serde(rename = "mdCodeBlockBorder")]
    pub md_code_block_border: ThemeColor,
    #[serde(rename = "mdQuote")]
    pub md_quote: ThemeColor,
    #[serde(rename = "mdQuoteBorder")]
    pub md_quote_border: ThemeColor,
    #[serde(rename = "mdHr")]
    pub md_hr: ThemeColor,
    #[serde(rename = "mdListBullet")]
    pub md_list_bullet: ThemeColor,

    // -- Diffs
    #[serde(rename = "toolDiffAdded")]
    pub tool_diff_added: ThemeColor,
    #[serde(rename = "toolDiffRemoved")]
    pub tool_diff_removed: ThemeColor,
    #[serde(rename = "toolDiffContext")]
    pub tool_diff_context: ThemeColor,

    // -- Syntax
    #[serde(rename = "syntaxComment")]
    pub syntax_comment: ThemeColor,
    #[serde(rename = "syntaxKeyword")]
    pub syntax_keyword: ThemeColor,
    #[serde(rename = "syntaxFunction")]
    pub syntax_function: ThemeColor,
    #[serde(rename = "syntaxVariable")]
    pub syntax_variable: ThemeColor,
    #[serde(rename = "syntaxString")]
    pub syntax_string: ThemeColor,
    #[serde(rename = "syntaxNumber")]
    pub syntax_number: ThemeColor,
    #[serde(rename = "syntaxType")]
    pub syntax_type: ThemeColor,
    #[serde(rename = "syntaxOperator")]
    pub syntax_operator: ThemeColor,
    #[serde(rename = "syntaxPunctuation")]
    pub syntax_punctuation: ThemeColor,

    // -- Thinking levels
    #[serde(rename = "thinkingOff")]
    pub thinking_off: ThemeColor,
    #[serde(rename = "thinkingMinimal")]
    pub thinking_minimal: ThemeColor,
    #[serde(rename = "thinkingLow")]
    pub thinking_low: ThemeColor,
    #[serde(rename = "thinkingMedium")]
    pub thinking_medium: ThemeColor,
    #[serde(rename = "thinkingHigh")]
    pub thinking_high: ThemeColor,
    #[serde(rename = "thinkingXhigh")]
    pub thinking_xhigh: ThemeColor,

    // -- Bash mode
    #[serde(rename = "bashMode")]
    pub bash_mode: ThemeColor,

    // -- Extended tokens (optional — auto-derived from core palette when absent)

    /// Border character style: "rounded", "thick", "plain", or "double".
    #[serde(default, rename = "borderStyle", skip_serializing_if = "Option::is_none")]
    pub border_style_hint: Option<String>,

    /// Spinner accent color (used to auto-generate the 4-step gradient).
    #[serde(default, rename = "spinnerAccent", skip_serializing_if = "Option::is_none")]
    pub spinner_accent: Option<ThemeColor>,

    /// Context-bar segment overrides (optional — derived from core palette).
    #[serde(default, rename = "ctxBarSystem", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_system: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarNativeTools", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_native_tools: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarMcpTools", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_mcp_tools: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarMemory", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_memory: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarSkills", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_skills: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarMessages", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_messages: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarFree", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_free: Option<ThemeColor>,
    #[serde(default, rename = "ctxBarBuffer", skip_serializing_if = "Option::is_none")]
    pub ctx_bar_buffer: Option<ThemeColor>,
}

/// A loaded theme ready for use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// Short human-readable description (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Theme author (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Colour-scheme variant: "dark", "light", or "auto" (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Optional variable aliases (referenced by name in `colors`).
    #[serde(default)]
    pub vars: HashMap<String, ThemeColor>,
    pub colors: ThemeTokens,
    /// Source file path (not serialized).
    #[serde(skip)]
    pub source: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorDef {
    Rgb(u8, u8, u8),
    Reset,
}

impl ColorDef {
    /// Serde default helper — used by `#[serde(default = "ColorDef::default_reset")]`.
    pub fn default_reset() -> Self {
        Self::Reset
    }
}

/// Which ratatui `BorderType` the theme prefers for all overlay blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BorderStyle {
    #[default]
    Rounded,
    Thick,
    Plain,
    Double,
}

/// Resolved, Agnostic color palette for the TUI.
///
/// Populated from a user-supplied JSON theme (via
/// `cade_core::resources::themes::Theme`) or from the built-in defaults.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeColors {
    // -- Core
    pub source_path: Option<std::path::PathBuf>,

    // -- Semantic Palette (Phase 1)
    pub bg_base: ColorDef,
    pub bg_surface0: ColorDef,
    pub bg_surface1: ColorDef,
    pub bg_surface2: ColorDef,

    pub primary: ColorDef,
    pub success: ColorDef,
    pub error: ColorDef,
    pub warning: ColorDef,

    pub text_primary: ColorDef,
    pub text_muted: ColorDef,
    pub text_dim: ColorDef,

    pub border_base: ColorDef,
    pub border_focus: ColorDef,

    // -- Diffs
    pub diff_added: ColorDef,
    pub diff_removed: ColorDef,
    pub diff_context: ColorDef,

    // -- Markdown
    pub md_heading: ColorDef,
    pub md_link: ColorDef,
    pub md_link_url: ColorDef,
    pub md_code: ColorDef,
    pub md_code_block: ColorDef,
    pub md_code_block_border: ColorDef,
    pub md_quote: ColorDef,
    pub md_quote_border: ColorDef,
    pub md_hr: ColorDef,
    pub md_list_bullet: ColorDef,

    // -- Syntax highlighting
    pub syntax_comment: ColorDef,
    pub syntax_keyword: ColorDef,
    pub syntax_function: ColorDef,
    pub syntax_variable: ColorDef,
    pub syntax_string: ColorDef,
    pub syntax_number: ColorDef,
    pub syntax_type: ColorDef,
    pub syntax_operator: ColorDef,
    pub syntax_punctuation: ColorDef,
    pub syntax_constant: ColorDef,
    pub syntax_string_escape: ColorDef,
    pub syntax_type_builtin: ColorDef,
    pub syntax_keyword_control: ColorDef,
    pub syntax_keyword_operator: ColorDef,
    pub syntax_entity_name_function: ColorDef,
    pub syntax_entity_name_type: ColorDef,
    pub syntax_variable_parameter: ColorDef,
    pub syntax_variable_other_member: ColorDef,
    pub syntax_support_function: ColorDef,
    pub syntax_support_macro: ColorDef,

    // -- Thinking level borders
    pub thinking_off: ColorDef,
    pub thinking_minimal: ColorDef,
    pub thinking_low: ColorDef,
    pub thinking_medium: ColorDef,
    pub thinking_high: ColorDef,
    pub thinking_xhigh: ColorDef,

    // -- Bash mode editor border
    pub bash_mode: ColorDef,

    // -- Extended surface tokens (Step 1)
    /// Preferred border character style for all overlay blocks.
    pub border_style: BorderStyle,
    /// Subtle card background for tool-result / message cards.
    pub bg_card: ColorDef,
    /// Input area background (textarea row).
    pub bg_input: ColorDef,
    /// Desaturated accent for secondary emphasis (badges, counts).
    pub accent_dim: ColorDef,

    // -- Context-bar category colors (data-viz)
    /// System prompt segment in the context usage bar.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_system: ColorDef,
    /// Native tools segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_native_tools: ColorDef,
    /// MCP tools segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_mcp_tools: ColorDef,
    /// Memory segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_memory: ColorDef,
    /// Skills segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_skills: ColorDef,
    /// Messages segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_messages: ColorDef,
    /// Free/unused segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_free: ColorDef,
    /// Buffer (autocompact) segment.
    #[serde(default = "ColorDef::default_reset")]
    pub ctx_bar_buffer: ColorDef,

    // -- Animated spinner gradient (4 steps)
    #[serde(default = "ColorDef::default_reset")]
    pub spinner_0: ColorDef,
    #[serde(default = "ColorDef::default_reset")]
    pub spinner_1: ColorDef,
    #[serde(default = "ColorDef::default_reset")]
    pub spinner_2: ColorDef,
    #[serde(default = "ColorDef::default_reset")]
    pub spinner_3: ColorDef,
    // -- Tool result status backgrounds
    pub selected_bg: ColorDef,
    pub tool_pending_bg: ColorDef,
    pub tool_success_bg: ColorDef,
    pub tool_error_bg: ColorDef,
}

impl ThemeColors {
    // -- Built-in themes

    /// Resolve a built-in theme name to `ThemeColors`.
    ///
    /// Returns `None` if `name` is not a recognised built-in.  Call sites that
    /// want fallback behaviour should use `.unwrap_or_else(Self::dark)` or
    /// cascade to [`discover_themes`] + [`Self::from_theme`] for custom themes.
    ///
    /// This is the single source of truth for the built-in theme registry —
    /// the CLI, TUI picker, GUI palette, and server `/theme` handler all
    /// delegate here so the list cannot drift.
    pub fn builtin_by_name(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            "catppuccin-mocha" => Some(Self::catppuccin_mocha()),
            "catppuccin-latte" => Some(Self::catppuccin_latte()),
            "tokyo-night" => Some(Self::tokyo_night()),
            _ => None,
        }
    }

    /// List all built-in theme names in display order.
    pub fn builtin_names() -> &'static [&'static str] {
        &[
            "dark",
            "light",
            "catppuccin-mocha",
            "catppuccin-latte",
            "tokyo-night",
        ]
    }

    /// Metadata for every built-in theme (name, description, variant).
    /// Used to populate pickers without fabricating phantom `Theme` structs.
    pub fn builtin_listing() -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("dark", "Built-in dark theme", "dark"),
            ("light", "Built-in light theme", "light"),
            ("catppuccin-mocha", "Catppuccin Mocha (dark pastel)", "dark"),
            (
                "catppuccin-latte",
                "Catppuccin Latte (light pastel)",
                "light",
            ),
            ("tokyo-night", "Tokyo Night (dark neon)", "dark"),
        ]
    }

    /// Dark theme (deep blue-black, high contrast, rich depth).
    pub fn dark() -> Self {
        Self {
            source_path: None,

            // Semantic Elevation — noticeable depth between layers
            bg_base: ColorDef::Rgb(12, 13, 20), // near-void blue-black
            bg_surface0: ColorDef::Rgb(20, 22, 33), // card base  (+8 step)
            bg_surface1: ColorDef::Rgb(26, 28, 42), // overlay base (+6 step)
            bg_surface2: ColorDef::Rgb(34, 36, 54), // selection highlight (+8 step)

            primary: ColorDef::Rgb(122, 162, 247), // vivid sky blue
            success: ColorDef::Rgb(73, 196, 127),  // fresh green
            error: ColorDef::Rgb(247, 93, 100),    // soft coral
            warning: ColorDef::Rgb(224, 175, 104), // warm amber

            text_primary: ColorDef::Rgb(205, 214, 244), // near-white (Catppuccin "Text")
            text_muted: ColorDef::Rgb(122, 128, 153),   // mid-grey, blue-tinted
            text_dim: ColorDef::Rgb(72, 78, 98),        // dark hint text

            border_base: ColorDef::Rgb(41, 44, 64), // barely-visible divider
            border_focus: ColorDef::Rgb(122, 162, 247), // matches primary

            diff_added: ColorDef::Rgb(73, 196, 127),
            diff_removed: ColorDef::Rgb(247, 93, 100),
            diff_context: ColorDef::Rgb(90, 98, 120),

            md_heading: ColorDef::Rgb(224, 175, 104),
            md_link: ColorDef::Rgb(122, 162, 247),
            md_link_url: ColorDef::Rgb(122, 128, 153),
            md_code: ColorDef::Rgb(115, 218, 202),
            md_code_block: ColorDef::Reset,
            md_code_block_border: ColorDef::Rgb(48, 52, 72),
            md_quote: ColorDef::Rgb(122, 128, 153),
            md_quote_border: ColorDef::Rgb(48, 52, 72),
            md_hr: ColorDef::Rgb(48, 52, 72),
            md_list_bullet: ColorDef::Rgb(115, 218, 202),

            syntax_comment: ColorDef::Rgb(90, 98, 120),
            syntax_keyword: ColorDef::Rgb(187, 154, 247), // purple
            syntax_function: ColorDef::Rgb(122, 162, 247), // blue
            syntax_variable: ColorDef::Rgb(224, 175, 104), // amber
            syntax_string: ColorDef::Rgb(158, 206, 106),  // green
            syntax_number: ColorDef::Rgb(255, 158, 100),  // orange
            syntax_type: ColorDef::Rgb(115, 218, 202),    // teal
            syntax_operator: ColorDef::Rgb(187, 154, 247),
            syntax_punctuation: ColorDef::Rgb(122, 128, 153),
            syntax_constant: ColorDef::Rgb(255, 158, 100),
            syntax_string_escape: ColorDef::Rgb(158, 206, 106),
            syntax_type_builtin: ColorDef::Rgb(115, 218, 202),
            syntax_keyword_control: ColorDef::Rgb(187, 154, 247),
            syntax_keyword_operator: ColorDef::Rgb(187, 154, 247),
            syntax_entity_name_function: ColorDef::Rgb(122, 162, 247),
            syntax_entity_name_type: ColorDef::Rgb(115, 218, 202),
            syntax_variable_parameter: ColorDef::Rgb(224, 175, 104),
            syntax_variable_other_member: ColorDef::Rgb(224, 175, 104),
            syntax_support_function: ColorDef::Rgb(122, 162, 247),
            syntax_support_macro: ColorDef::Rgb(122, 162, 247),

            thinking_off: ColorDef::Rgb(41, 44, 64),
            thinking_minimal: ColorDef::Rgb(122, 162, 247),
            thinking_low: ColorDef::Rgb(80, 160, 200),
            thinking_medium: ColorDef::Rgb(115, 218, 202),
            thinking_high: ColorDef::Rgb(224, 175, 104),
            thinking_xhigh: ColorDef::Rgb(247, 93, 100),
            bash_mode: ColorDef::Rgb(224, 175, 104),

            border_style: BorderStyle::Rounded,
            bg_card: ColorDef::Rgb(20, 22, 34),
            bg_input: ColorDef::Rgb(22, 24, 36),
            accent_dim: ColorDef::Rgb(64, 102, 168),

            // Context-bar data-viz
            ctx_bar_system: ColorDef::Rgb(120, 120, 120),
            ctx_bar_native_tools: ColorDef::Rgb(8, 145, 178),
            ctx_bar_mcp_tools: ColorDef::Rgb(0, 188, 212),
            ctx_bar_memory: ColorDef::Rgb(215, 119, 87),
            ctx_bar_skills: ColorDef::Rgb(255, 193, 7),
            ctx_bar_messages: ColorDef::Rgb(147, 51, 234),
            ctx_bar_free: ColorDef::Rgb(50, 50, 50),
            ctx_bar_buffer: ColorDef::Rgb(80, 80, 80),

            // Animated spinner gradient
            spinner_0: ColorDef::Rgb(80, 190, 255),
            spinner_1: ColorDef::Rgb(120, 215, 255),
            spinner_2: ColorDef::Rgb(160, 235, 255),
            spinner_3: ColorDef::Rgb(100, 200, 255),
            // Tool result status backgrounds (dark theme)
            selected_bg: ColorDef::Rgb(30, 33, 50),
            tool_pending_bg: ColorDef::Rgb(25, 30, 45),
            tool_success_bg: ColorDef::Rgb(20, 35, 25),
            tool_error_bg: ColorDef::Rgb(40, 20, 22),
        }
    }

    /// Resolve a full `Theme` into a runtime `ThemeColors`.
    ///
    /// Maps every `ThemeTokens` field to the corresponding semantic slot in
    /// `ThemeColors`.  Unmapped fields fall back to the `dark()` defaults.
    ///
    /// The mapping is deliberately dense — imported VS Code / Sublime /
    /// TextMate themes populate `ThemeTokens` via `load_theme()` / `.tmTheme`
    /// parsing, and we want those to render fully, not just accent + bg.
    pub fn from_theme(theme: &Theme) -> ThemeColors {
        let mut base = Self::dark();
        base.source_path = Some(theme.source.clone());

        let resolve = |c: &ThemeColor| -> ColorDef { resolve_color(c, &theme.vars) };
        let t = &theme.colors;

        // -- Primary / semantic
        base.primary = resolve(&t.accent);
        base.success = resolve(&t.success);
        base.error = resolve(&t.error);
        base.warning = resolve(&t.warning);
        // Accent-dim: re-use accent (user themes rarely define a dim variant)
        base.accent_dim = resolve(&t.accent);

        // -- Backgrounds (semantic elevations)
        base.bg_base = resolve(&t.custom_message_bg);
        base.bg_surface0 = resolve(&t.user_message_bg);
        base.bg_surface1 = resolve(&t.tool_pending_bg);
        base.bg_surface2 = resolve(&t.selected_bg);
        base.bg_card = resolve(&t.tool_success_bg);
        base.bg_input = resolve(&t.user_message_bg);
        base.selected_bg = resolve(&t.selected_bg);
        base.tool_pending_bg = resolve(&t.tool_pending_bg);
        base.tool_success_bg = resolve(&t.tool_success_bg);
        base.tool_error_bg = resolve(&t.tool_error_bg);

        // -- Borders
        base.border_base = resolve(&t.border);
        base.border_focus = resolve(&t.border_accent);

        // -- Text
        base.text_primary = resolve(&t.text);
        base.text_muted = resolve(&t.muted);
        base.text_dim = resolve(&t.dim);

        // -- Diffs
        base.diff_added = resolve(&t.tool_diff_added);
        base.diff_removed = resolve(&t.tool_diff_removed);
        base.diff_context = resolve(&t.tool_diff_context);

        // -- Markdown
        base.md_heading = resolve(&t.md_heading);
        base.md_link = resolve(&t.md_link);
        base.md_link_url = resolve(&t.md_link_url);
        base.md_code = resolve(&t.md_code);
        base.md_code_block = resolve(&t.md_code_block);
        base.md_code_block_border = resolve(&t.md_code_block_border);
        base.md_quote = resolve(&t.md_quote);
        base.md_quote_border = resolve(&t.md_quote_border);
        base.md_hr = resolve(&t.md_hr);
        base.md_list_bullet = resolve(&t.md_list_bullet);

        // -- Syntax (primary nine)
        base.syntax_comment = resolve(&t.syntax_comment);
        base.syntax_keyword = resolve(&t.syntax_keyword);
        base.syntax_function = resolve(&t.syntax_function);
        base.syntax_variable = resolve(&t.syntax_variable);
        base.syntax_string = resolve(&t.syntax_string);
        base.syntax_number = resolve(&t.syntax_number);
        base.syntax_type = resolve(&t.syntax_type);
        base.syntax_operator = resolve(&t.syntax_operator);
        base.syntax_punctuation = resolve(&t.syntax_punctuation);

        // -- Syntax (extended — reuse the closest ThemeTokens field since
        //    the JSON schema doesn't expose all granular slots).  These are
        //    the fine-grained TextMate scopes we approximate.
        base.syntax_constant = resolve(&t.syntax_number);
        base.syntax_string_escape = resolve(&t.syntax_string);
        base.syntax_type_builtin = resolve(&t.syntax_type);
        base.syntax_keyword_control = resolve(&t.syntax_keyword);
        base.syntax_keyword_operator = resolve(&t.syntax_operator);
        base.syntax_entity_name_function = resolve(&t.syntax_function);
        base.syntax_entity_name_type = resolve(&t.syntax_type);
        base.syntax_variable_parameter = resolve(&t.syntax_variable);
        base.syntax_variable_other_member = resolve(&t.syntax_variable);
        base.syntax_support_function = resolve(&t.syntax_function);
        base.syntax_support_macro = resolve(&t.syntax_function);

        // -- Reasoning / thinking tiers
        base.thinking_off = resolve(&t.thinking_off);
        base.thinking_minimal = resolve(&t.thinking_minimal);
        base.thinking_low = resolve(&t.thinking_low);
        base.thinking_medium = resolve(&t.thinking_medium);
        base.thinking_high = resolve(&t.thinking_high);
        base.thinking_xhigh = resolve(&t.thinking_xhigh);

        // -- Bash mode indicator
        base.bash_mode = resolve(&t.bash_mode);

        // -- Gap 3 fix: Fallback for missing thinkingXhigh / bashMode tokens.
        //    When the JSON theme omits these (serde default → empty string → Reset),
        //    derive sensible values from existing palette colors.
        if base.thinking_xhigh == ColorDef::Reset {
            base.thinking_xhigh = base.error;
        }
        if base.bash_mode == ColorDef::Reset {
            base.bash_mode = base.warning;
        }

        // -- Gap 1 fix: Auto-derive extended tokens from the theme's core palette.
        //    Custom JSON themes don't expose these, so we derive them here to avoid
        //    always falling back to Self::dark() defaults.

        // Border style hint
        if let Some(ref hint) = t.border_style_hint {
            base.border_style = match hint.as_str() {
                "thick" => BorderStyle::Thick,
                "plain" => BorderStyle::Plain,
                "double" => BorderStyle::Double,
                _ => BorderStyle::Rounded,
            };
        }

        // Spinner gradient: derive from spinner_accent or fall back to accent/primary.
        let spinner_base = t
            .spinner_accent
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.primary);
        if let ColorDef::Rgb(_, _, _) = spinner_base {
            // Build a 4-step luminance gradient: base → brighter → brightest → mid
            base.spinner_0 = spinner_base;
            base.spinner_1 = brighten_color(spinner_base, 30);
            base.spinner_2 = brighten_color(spinner_base, 50);
            base.spinner_3 = brighten_color(spinner_base, 15);
        }

        // Accent-dim: desaturate the primary slightly toward grey
        if let ColorDef::Rgb(r, g, b) = base.primary {
            let mix = |c: u8, grey: u8| -> u8 {
                ((c as u16 + grey as u16) / 2) as u8
            };
            base.accent_dim = ColorDef::Rgb(mix(r, 128), mix(g, 128), mix(b, 128));
        }

        // Context-bar: use explicit overrides if provided, otherwise derive from palette.
        base.ctx_bar_system = t
            .ctx_bar_system
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or_else(|| dim_color(base.text_muted, 30));
        base.ctx_bar_native_tools = t
            .ctx_bar_native_tools
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.primary);
        base.ctx_bar_mcp_tools = t
            .ctx_bar_mcp_tools
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or_else(|| brighten_color(base.primary, 30));
        base.ctx_bar_memory = t
            .ctx_bar_memory
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.warning);
        base.ctx_bar_skills = t
            .ctx_bar_skills
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or_else(|| brighten_color(base.warning, 20));
        base.ctx_bar_messages = t
            .ctx_bar_messages
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.syntax_keyword);
        base.ctx_bar_free = t
            .ctx_bar_free
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.text_dim);
        base.ctx_bar_buffer = t
            .ctx_bar_buffer
            .as_ref()
            .map(|c| resolve(c))
            .unwrap_or(base.border_base);

        base
    }

    /// Light theme (warm white base, high readability, clear depth).
    pub fn light() -> Self {
        Self {
            source_path: None,

            // Semantic Elevation — clear layering on a white surface
            bg_base: ColorDef::Rgb(252, 252, 255), // near-white, slight blue
            bg_surface0: ColorDef::Rgb(244, 246, 255), // card base
            bg_surface1: ColorDef::Rgb(236, 240, 255), // overlay / panel
            bg_surface2: ColorDef::Rgb(220, 226, 248), // selection highlight

            primary: ColorDef::Rgb(14, 98, 200), // rich blue
            success: ColorDef::Rgb(0, 135, 75),  // forest green
            error: ColorDef::Rgb(185, 28, 28),   // deep red
            warning: ColorDef::Rgb(146, 88, 0),  // dark amber

            text_primary: ColorDef::Rgb(30, 36, 58), // dark charcoal — readable on white
            text_muted: ColorDef::Rgb(90, 100, 125),
            text_dim: ColorDef::Rgb(148, 158, 180),

            border_base: ColorDef::Rgb(198, 206, 226),
            border_focus: ColorDef::Rgb(14, 98, 200),

            diff_added: ColorDef::Rgb(0, 135, 75),
            diff_removed: ColorDef::Rgb(185, 28, 28),
            diff_context: ColorDef::Rgb(90, 100, 125),

            md_heading: ColorDef::Rgb(146, 88, 0),
            md_link: ColorDef::Rgb(14, 98, 200),
            md_link_url: ColorDef::Rgb(90, 100, 125),
            md_code: ColorDef::Rgb(0, 118, 118),
            md_code_block: ColorDef::Rgb(30, 36, 58), // explicit dark (not Reset)
            md_code_block_border: ColorDef::Rgb(180, 190, 215),
            md_quote: ColorDef::Rgb(90, 100, 125),
            md_quote_border: ColorDef::Rgb(180, 190, 215),
            md_hr: ColorDef::Rgb(180, 190, 215),
            md_list_bullet: ColorDef::Rgb(0, 118, 118),

            syntax_comment: ColorDef::Rgb(90, 100, 125),
            syntax_keyword: ColorDef::Rgb(128, 30, 155),
            syntax_function: ColorDef::Rgb(14, 98, 200),
            syntax_variable: ColorDef::Rgb(146, 88, 0),
            syntax_string: ColorDef::Rgb(0, 115, 55),
            syntax_number: ColorDef::Rgb(155, 70, 15),
            syntax_type: ColorDef::Rgb(0, 95, 155),
            syntax_operator: ColorDef::Rgb(128, 30, 155),
            syntax_punctuation: ColorDef::Rgb(90, 100, 125),
            syntax_constant: ColorDef::Rgb(155, 70, 15),
            syntax_string_escape: ColorDef::Rgb(0, 115, 55),
            syntax_type_builtin: ColorDef::Rgb(0, 95, 155),
            syntax_keyword_control: ColorDef::Rgb(128, 30, 155),
            syntax_keyword_operator: ColorDef::Rgb(128, 30, 155),
            syntax_entity_name_function: ColorDef::Rgb(14, 98, 200),
            syntax_entity_name_type: ColorDef::Rgb(0, 95, 155),
            syntax_variable_parameter: ColorDef::Rgb(146, 88, 0),
            syntax_variable_other_member: ColorDef::Rgb(146, 88, 0),
            syntax_support_function: ColorDef::Rgb(14, 98, 200),
            syntax_support_macro: ColorDef::Rgb(14, 98, 200),

            thinking_off: ColorDef::Rgb(198, 206, 226),
            thinking_minimal: ColorDef::Rgb(14, 98, 200),
            thinking_low: ColorDef::Rgb(0, 118, 160),
            thinking_medium: ColorDef::Rgb(0, 135, 75),
            thinking_high: ColorDef::Rgb(146, 88, 0),
            thinking_xhigh: ColorDef::Rgb(185, 28, 28),
            bash_mode: ColorDef::Rgb(146, 88, 0),

            border_style: BorderStyle::Rounded,
            bg_card: ColorDef::Rgb(242, 245, 255),
            bg_input: ColorDef::Rgb(236, 240, 255),
            accent_dim: ColorDef::Rgb(76, 136, 210),

            // Context-bar data-viz (lighter variants for readability on white)
            ctx_bar_system: ColorDef::Rgb(140, 140, 140),
            ctx_bar_native_tools: ColorDef::Rgb(0, 130, 165),
            ctx_bar_mcp_tools: ColorDef::Rgb(0, 160, 190),
            ctx_bar_memory: ColorDef::Rgb(195, 100, 70),
            ctx_bar_skills: ColorDef::Rgb(200, 155, 0),
            ctx_bar_messages: ColorDef::Rgb(130, 40, 210),
            ctx_bar_free: ColorDef::Rgb(210, 210, 210),
            ctx_bar_buffer: ColorDef::Rgb(180, 180, 180),

            // Animated spinner gradient (darker for light bg)
            spinner_0: ColorDef::Rgb(20, 130, 210),
            spinner_1: ColorDef::Rgb(40, 150, 220),
            spinner_2: ColorDef::Rgb(60, 170, 230),
            spinner_3: ColorDef::Rgb(30, 140, 215),
            // Tool result status backgrounds (light theme)
            selected_bg: ColorDef::Rgb(220, 225, 240),
            tool_pending_bg: ColorDef::Rgb(230, 238, 245),
            tool_success_bg: ColorDef::Rgb(225, 245, 230),
            tool_error_bg: ColorDef::Rgb(250, 225, 225),
        }
    }

    // -- Additional built-in themes

    /// Catppuccin Mocha — warm purple-tinted dark.
    /// Palette source: <https://github.com/catppuccin/catppuccin>
    pub fn catppuccin_mocha() -> Self {
        let mut c = Self::dark();
        // Base surfaces
        c.bg_base = ColorDef::Rgb(30, 30, 46); // Crust
        c.bg_surface0 = ColorDef::Rgb(36, 36, 54); // Mantle
        c.bg_surface1 = ColorDef::Rgb(49, 50, 68); // Base
        c.bg_surface2 = ColorDef::Rgb(69, 71, 90); // Surface0
        c.bg_card = ColorDef::Rgb(36, 36, 54);
        c.bg_input = ColorDef::Rgb(49, 50, 68);
        // Accents
        c.primary = ColorDef::Rgb(137, 180, 250); // Blue
        c.success = ColorDef::Rgb(166, 227, 161); // Green
        c.error = ColorDef::Rgb(243, 139, 168); // Red
        c.warning = ColorDef::Rgb(249, 226, 175); // Yellow
        c.accent_dim = ColorDef::Rgb(88, 128, 200);
        // Text
        c.text_muted = ColorDef::Rgb(166, 173, 200); // Overlay2
        c.text_dim = ColorDef::Rgb(108, 112, 134); // Surface2
        // Borders
        c.border_base = ColorDef::Rgb(69, 71, 90);
        c.border_focus = ColorDef::Rgb(137, 180, 250);
        // Diff
        c.diff_added = ColorDef::Rgb(166, 227, 161);
        c.diff_removed = ColorDef::Rgb(243, 139, 168);
        // Markdown
        c.md_heading = ColorDef::Rgb(249, 226, 175);
        c.md_link = ColorDef::Rgb(137, 180, 250);
        c.md_code = ColorDef::Rgb(148, 226, 213); // Teal
        c.md_list_bullet = ColorDef::Rgb(148, 226, 213);
        // Syntax
        c.syntax_keyword = ColorDef::Rgb(203, 166, 247); // Mauve
        c.syntax_function = ColorDef::Rgb(137, 180, 250); // Blue
        c.syntax_string = ColorDef::Rgb(166, 227, 161); // Green
        c.syntax_number = ColorDef::Rgb(250, 179, 135); // Peach
        c.syntax_type = ColorDef::Rgb(148, 226, 213); // Teal
        c.syntax_variable = ColorDef::Rgb(249, 226, 175); // Yellow
        c.syntax_operator = ColorDef::Rgb(203, 166, 247);
        // Thinking
        c.thinking_minimal = ColorDef::Rgb(137, 180, 250);
        c.thinking_medium = ColorDef::Rgb(148, 226, 213);
        c.thinking_high = ColorDef::Rgb(249, 226, 175);
        c.thinking_xhigh = ColorDef::Rgb(243, 139, 168);
        c.bash_mode = ColorDef::Rgb(249, 226, 175);

        c.syntax_constant = c.syntax_number;
        c.syntax_string_escape = c.syntax_string;
        c.syntax_type_builtin = c.syntax_type;
        c.syntax_keyword_control = c.syntax_keyword;
        c.syntax_keyword_operator = c.syntax_operator;
        c.syntax_entity_name_function = c.syntax_function;
        c.syntax_entity_name_type = c.syntax_type;
        c.syntax_variable_parameter = c.syntax_variable;
        c.syntax_variable_other_member = c.syntax_variable;
        c.syntax_support_function = c.syntax_function;
        c.syntax_support_macro = c.syntax_function;

        // Context-bar (catppuccin mocha palette-aligned)
        c.ctx_bar_system = ColorDef::Rgb(108, 112, 134); // Surface2
        c.ctx_bar_native_tools = ColorDef::Rgb(116, 199, 236); // Sapphire
        c.ctx_bar_mcp_tools = ColorDef::Rgb(137, 220, 235); // Sky
        c.ctx_bar_memory = ColorDef::Rgb(250, 179, 135); // Peach
        c.ctx_bar_skills = ColorDef::Rgb(249, 226, 175); // Yellow
        c.ctx_bar_messages = ColorDef::Rgb(203, 166, 247); // Mauve
        c.ctx_bar_free = ColorDef::Rgb(49, 50, 68); // Surface0
        c.ctx_bar_buffer = ColorDef::Rgb(69, 71, 90); // Surface1

        // Spinner gradient (blue tints from catppuccin)
        c.spinner_0 = ColorDef::Rgb(116, 199, 236); // Sapphire
        c.spinner_1 = ColorDef::Rgb(137, 220, 235); // Sky
        c.spinner_2 = ColorDef::Rgb(148, 226, 213); // Teal
        c.spinner_3 = ColorDef::Rgb(137, 180, 250); // Blue

        c.selected_bg = ColorDef::Rgb(49, 50, 68);  // Surface1
        c.tool_pending_bg = ColorDef::Rgb(30, 30, 46); // Base
        c.tool_success_bg = ColorDef::Rgb(30, 40, 35);
        c.tool_error_bg = ColorDef::Rgb(50, 30, 35);

        c
    }
    /// Palette source: <https://github.com/catppuccin/catppuccin>
    pub fn catppuccin_latte() -> Self {
        let mut c = Self::light();
        // Base surfaces
        c.bg_base = ColorDef::Rgb(239, 241, 245); // Base
        c.bg_surface0 = ColorDef::Rgb(230, 233, 239); // Mantle
        c.bg_surface1 = ColorDef::Rgb(220, 224, 232); // Crust
        c.bg_surface2 = ColorDef::Rgb(204, 208, 218); // Surface0
        c.bg_card = ColorDef::Rgb(230, 233, 239);
        c.bg_input = ColorDef::Rgb(220, 224, 232);
        // Accents
        c.primary = ColorDef::Rgb(30, 102, 245); // Blue
        c.success = ColorDef::Rgb(64, 160, 43); // Green
        c.error = ColorDef::Rgb(210, 15, 57); // Red
        c.warning = ColorDef::Rgb(223, 142, 29); // Yellow
        c.accent_dim = ColorDef::Rgb(80, 140, 210);
        // Text
        c.text_primary = ColorDef::Rgb(76, 79, 105); // Text
        c.text_muted = ColorDef::Rgb(92, 106, 134); // Overlay2
        c.text_dim = ColorDef::Rgb(156, 160, 176); // Surface2
        // Borders
        c.border_base = ColorDef::Rgb(188, 192, 204);
        c.border_focus = ColorDef::Rgb(30, 102, 245);
        // Markdown
        c.md_heading = ColorDef::Rgb(223, 142, 29);
        c.md_link = ColorDef::Rgb(30, 102, 245);
        c.md_code = ColorDef::Rgb(23, 146, 153); // Teal
        c.md_list_bullet = ColorDef::Rgb(23, 146, 153);
        // Syntax
        c.syntax_keyword = ColorDef::Rgb(136, 57, 239); // Mauve
        c.syntax_function = ColorDef::Rgb(30, 102, 245);
        c.syntax_string = ColorDef::Rgb(64, 160, 43);
        c.syntax_number = ColorDef::Rgb(254, 100, 11); // Peach
        c.syntax_type = ColorDef::Rgb(23, 146, 153);
        c.syntax_variable = ColorDef::Rgb(223, 142, 29);
        c.syntax_operator = ColorDef::Rgb(136, 57, 239);
        c.bash_mode = ColorDef::Rgb(223, 142, 29);

        c.syntax_constant = c.syntax_number;
        c.syntax_string_escape = c.syntax_string;
        c.syntax_type_builtin = c.syntax_type;
        c.syntax_keyword_control = c.syntax_keyword;
        c.syntax_keyword_operator = c.syntax_operator;
        c.syntax_entity_name_function = c.syntax_function;
        c.syntax_entity_name_type = c.syntax_type;
        c.syntax_variable_parameter = c.syntax_variable;
        c.syntax_variable_other_member = c.syntax_variable;
        c.syntax_support_function = c.syntax_function;
        c.syntax_support_macro = c.syntax_function;

        // Context-bar (catppuccin latte palette-aligned)
        c.ctx_bar_system = ColorDef::Rgb(156, 160, 176); // Surface2
        c.ctx_bar_native_tools = ColorDef::Rgb(4, 165, 229); // Sapphire
        c.ctx_bar_mcp_tools = ColorDef::Rgb(2, 169, 165); // Sky  (latte sky ≈ teal)
        c.ctx_bar_memory = ColorDef::Rgb(254, 100, 11); // Peach
        c.ctx_bar_skills = ColorDef::Rgb(223, 142, 29); // Yellow
        c.ctx_bar_messages = ColorDef::Rgb(136, 57, 239); // Mauve
        c.ctx_bar_free = ColorDef::Rgb(220, 224, 232); // Crust
        c.ctx_bar_buffer = ColorDef::Rgb(204, 208, 218); // Surface0

        // Spinner gradient (blue tints from latte)
        c.spinner_0 = ColorDef::Rgb(30, 102, 245); // Blue
        c.spinner_1 = ColorDef::Rgb(4, 165, 229); // Sapphire
        c.spinner_2 = ColorDef::Rgb(23, 146, 153); // Teal
        c.spinner_3 = ColorDef::Rgb(2, 169, 165); // Sky

        c.selected_bg = ColorDef::Rgb(204, 208, 218);
        c.tool_pending_bg = ColorDef::Rgb(220, 224, 232);
        c.tool_success_bg = ColorDef::Rgb(210, 240, 215);
        c.tool_error_bg = ColorDef::Rgb(245, 215, 215);

        c
    }

    /// Tokyo Night — deep indigo dark with neon cyan + rose accents.
    /// Palette source: <https://github.com/enkia/tokyo-night-vscode-theme>
    pub fn tokyo_night() -> Self {
        let mut c = Self::dark();
        // Base surfaces
        c.bg_base = ColorDef::Rgb(26, 27, 38); // bg
        c.bg_surface0 = ColorDef::Rgb(28, 29, 44); // bg_dark
        c.bg_surface1 = ColorDef::Rgb(32, 34, 51); // bg_highlight
        c.bg_surface2 = ColorDef::Rgb(41, 44, 66); // terminal_black
        c.bg_card = ColorDef::Rgb(28, 29, 44);
        c.bg_input = ColorDef::Rgb(32, 34, 51);
        // Accents
        c.primary = ColorDef::Rgb(122, 162, 247); // blue
        c.success = ColorDef::Rgb(158, 206, 106); // green
        c.error = ColorDef::Rgb(247, 93, 100); // red
        c.warning = ColorDef::Rgb(224, 175, 104); // yellow
        c.accent_dim = ColorDef::Rgb(65, 105, 190);
        // Text
        c.text_muted = ColorDef::Rgb(169, 177, 214); // fg_dark
        c.text_dim = ColorDef::Rgb(86, 95, 137); // comment
        // Borders
        c.border_base = ColorDef::Rgb(41, 44, 66);
        c.border_focus = ColorDef::Rgb(122, 162, 247);
        // Diff
        c.diff_added = ColorDef::Rgb(158, 206, 106);
        c.diff_removed = ColorDef::Rgb(247, 93, 100);
        // Markdown
        c.md_heading = ColorDef::Rgb(224, 175, 104);
        c.md_link = ColorDef::Rgb(122, 162, 247);
        c.md_code = ColorDef::Rgb(42, 195, 222); // cyan
        c.md_list_bullet = ColorDef::Rgb(42, 195, 222);
        // Syntax
        c.syntax_keyword = ColorDef::Rgb(187, 154, 247); // purple
        c.syntax_function = ColorDef::Rgb(122, 162, 247); // blue
        c.syntax_string = ColorDef::Rgb(158, 206, 106); // green
        c.syntax_number = ColorDef::Rgb(255, 158, 100); // orange
        c.syntax_type = ColorDef::Rgb(42, 195, 222); // cyan
        c.syntax_variable = ColorDef::Rgb(224, 175, 104); // yellow
        c.syntax_operator = ColorDef::Rgb(187, 154, 247);
        // Thinking
        c.thinking_minimal = ColorDef::Rgb(122, 162, 247);
        c.thinking_medium = ColorDef::Rgb(42, 195, 222);
        c.thinking_high = ColorDef::Rgb(224, 175, 104);
        c.thinking_xhigh = ColorDef::Rgb(247, 93, 100);
        c.bash_mode = ColorDef::Rgb(224, 175, 104);

        c.syntax_constant = c.syntax_number;
        c.syntax_string_escape = c.syntax_string;
        c.syntax_type_builtin = c.syntax_type;
        c.syntax_keyword_control = c.syntax_keyword;
        c.syntax_keyword_operator = c.syntax_operator;
        c.syntax_entity_name_function = c.syntax_function;
        c.syntax_entity_name_type = c.syntax_type;
        c.syntax_variable_parameter = c.syntax_variable;
        c.syntax_variable_other_member = c.syntax_variable;
        c.syntax_support_function = c.syntax_function;
        c.syntax_support_macro = c.syntax_function;

        // Context-bar (tokyo-night palette-aligned)
        c.ctx_bar_system = ColorDef::Rgb(86, 95, 137); // comment
        c.ctx_bar_native_tools = ColorDef::Rgb(42, 195, 222); // cyan
        c.ctx_bar_mcp_tools = ColorDef::Rgb(115, 218, 202); // teal
        c.ctx_bar_memory = ColorDef::Rgb(255, 158, 100); // orange
        c.ctx_bar_skills = ColorDef::Rgb(224, 175, 104); // yellow
        c.ctx_bar_messages = ColorDef::Rgb(187, 154, 247); // magenta
        c.ctx_bar_free = ColorDef::Rgb(32, 34, 51); // bg_highlight
        c.ctx_bar_buffer = ColorDef::Rgb(41, 44, 66); // terminal_black

        // Spinner gradient (blue→cyan from tokyo-night)
        c.spinner_0 = ColorDef::Rgb(122, 162, 247); // blue
        c.spinner_1 = ColorDef::Rgb(125, 207, 255); // blue5
        c.spinner_2 = ColorDef::Rgb(42, 195, 222); // cyan
        c.spinner_3 = ColorDef::Rgb(115, 218, 202); // teal

        c.selected_bg = ColorDef::Rgb(41, 44, 66);
        c.tool_pending_bg = ColorDef::Rgb(30, 32, 48);
        c.tool_success_bg = ColorDef::Rgb(28, 40, 35);
        c.tool_error_bg = ColorDef::Rgb(50, 28, 32);

        c
    }
}

// endregion: --- Types

// region:    --- Discovery

/// Discover all custom themes from standard locations.
/// Project-local themes shadow global ones with the same name.
pub fn discover_themes(cwd: &Path, agent_dir: &Path) -> Vec<Theme> {
    let mut themes: Vec<Theme> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Project-local (higher priority)
    let project_dir = cwd.join(".cade").join("themes");
    load_themes_from_dir(&project_dir, &mut themes, &mut seen);

    // Global
    let global_dir = agent_dir.join("themes");
    load_themes_from_dir(&global_dir, &mut themes, &mut seen);

    themes
}

/// Discover all themes (built-ins merged with on-disk) in display order.
///
/// Built-ins from [`ThemeColors::builtin_listing`] are included at the top
/// unless an on-disk file of the same name has already been discovered.
/// The stubs carry `colors: Default::default()` and `source: "builtin"` —
/// callers that want to resolve colors for a built-in must call
/// [`ThemeColors::builtin_by_name`] first.
pub fn discover_themes_with_builtins(cwd: &Path, agent_dir: &Path) -> Vec<Theme> {
    let mut themes = discover_themes(cwd, agent_dir);

    for (idx, (name, desc, variant)) in ThemeColors::builtin_listing().iter().enumerate() {
        if !themes.iter().any(|t| t.name == *name) {
            themes.insert(
                idx.min(themes.len()),
                Theme {
                    name: name.to_string(),
                    description: Some(desc.to_string()),
                    author: Some("CADE".to_string()),
                    variant: Some(variant.to_string()),
                    vars: Default::default(),
                    colors: ThemeTokens::default(),
                    source: std::path::PathBuf::from("builtin"),
                },
            );
        }
    }

    themes
}

/// Load a single theme from a JSON or tmTheme file.
pub fn load_theme(path: &Path) -> crate::Result<Theme> {
    if path.extension().and_then(|e| e.to_str()) == Some("tmTheme") {
        return load_tmtheme(path);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::custom(format!("read theme {}: {e}", path.display())))?;
    let mut theme: Theme = serde_json::from_str(&content)
        .map_err(|e| crate::Error::custom(format!("parse theme {}: {e}", path.display())))?;
    theme.source = path.to_path_buf();
    Ok(theme)
}

fn load_tmtheme(path: &Path) -> crate::Result<Theme> {
    let plist_value = plist::Value::from_file(path)
        .map_err(|e| crate::Error::custom(format!("parse tmTheme {}: {e}", path.display())))?;

    let dict = plist_value
        .into_dictionary()
        .ok_or_else(|| crate::Error::custom("root is not a dictionary"))?;

    let name = dict
        .get("name")
        .and_then(|v| v.as_string())
        .unwrap_or("unnamed")
        .to_string();

    let mut theme = Theme {
        name,
        description: None,
        author: None,
        variant: None,
        vars: HashMap::new(),
        colors: ThemeTokens::default(),
        source: path.to_path_buf(),
    };

    let settings_array = dict
        .get("settings")
        .and_then(|v| v.as_array())
        .ok_or_else(|| crate::Error::custom("missing settings array"))?;

    let mut global_settings = None;
    let mut scopes = HashMap::new();

    for item in settings_array {
        if let Some(item_dict) = item.as_dictionary() {
            let item_settings = item_dict.get("settings").and_then(|v| v.as_dictionary());
            if let Some(settings) = item_settings {
                if let Some(scope) = item_dict.get("scope").and_then(|v| v.as_string()) {
                    scopes.insert(scope.to_string(), settings.clone());
                } else if global_settings.is_none() {
                    global_settings = Some(settings.clone());
                }
            }
        }
    }

    let to_color = |v: Option<&plist::Value>| -> Option<ThemeColor> {
        v.and_then(|val| val.as_string())
            .map(|s| ThemeColor::Hex(s.to_string()))
    };

    let mut fallback_bg = ThemeColor::default();
    let mut fallback_fg = ThemeColor::default();

    if let Some(global) = global_settings {
        if let Some(bg) = to_color(global.get("background")) {
            theme.colors.user_message_bg = bg.clone();
            theme.colors.tool_pending_bg = bg.clone();
            theme.colors.bash_mode = bg.clone();
            fallback_bg = bg.clone();
        }
        if let Some(fg) = to_color(global.get("foreground")) {
            theme.colors.text = fg.clone();
            fallback_fg = fg.clone();
        }
        if let Some(sel) = to_color(global.get("selection")) {
            theme.colors.selected_bg = sel.clone();
            theme.colors.tool_diff_context = sel.clone();
        }
        if let Some(line_hl) = to_color(global.get("lineHighlight")) {
            theme.colors.border = line_hl.clone();
            theme.colors.dim = line_hl.clone();
            theme.colors.muted = line_hl.clone();
        } else {
            theme.colors.border = fallback_bg.clone();
            theme.colors.dim = fallback_bg.clone();
            theme.colors.muted = fallback_bg.clone();
        }
    }

    let get_scope_fg = |scope_str: &str| -> Option<ThemeColor> {
        for (scope, settings) in &scopes {
            if scope.split(',').any(|s| s.trim().starts_with(scope_str)) {
                return to_color(settings.get("foreground"));
            }
        }
        None
    };

    if let Some(accent) = get_scope_fg("keyword.control") {
        theme.colors.accent = accent.clone();
        theme.colors.tool_title = accent;
    } else {
        theme.colors.accent = fallback_fg.clone();
    }

    if let Some(success) = get_scope_fg("string") {
        theme.colors.success = success.clone();
        theme.colors.tool_success_bg = success;
    }

    if let Some(warning) = get_scope_fg("entity.name.function") {
        theme.colors.warning = warning;
    }

    if let Some(error) = get_scope_fg("invalid") {
        theme.colors.error = error.clone();
        theme.colors.tool_error_bg = error;
    }

    if let Some(link) = get_scope_fg("markup.underline.link") {
        theme.colors.md_link = link;
    }

    Ok(theme)
}

// endregion: --- Discovery

// region:    --- Support

fn load_themes_from_dir(
    dir: &Path,
    themes: &mut Vec<Theme>,
    seen: &mut std::collections::HashSet<String>,
) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json")
            && path.extension().and_then(|e| e.to_str()) != Some("tmTheme")
        {
            continue;
        }
        match load_theme(&path) {
            Ok(theme) => {
                if !seen.contains(&theme.name) {
                    seen.insert(theme.name.clone());
                    themes.push(theme);
                }
            }
            Err(e) => tracing::warn!("Skipping theme {}: {e}", path.display()),
        }
    }
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_discover_themes_empty() {
        // -- Setup & Fixtures
        let cwd = make_dir();
        let agent_dir = make_dir();

        // -- Exec
        let themes = discover_themes(cwd.path(), agent_dir.path());

        // -- Check
        assert!(themes.is_empty());
    }

    // -- Built-in registry tests (Phase 1)

    #[test]
    fn builtin_by_name_resolves_all_known_themes() {
        for name in ThemeColors::builtin_names() {
            let tc = ThemeColors::builtin_by_name(name)
                .unwrap_or_else(|| panic!("builtin '{name}' must resolve"));
            assert_ne!(
                tc.primary,
                ColorDef::Reset,
                "builtin '{name}' primary must not be Reset"
            );
        }
    }

    #[test]
    fn builtin_by_name_returns_none_for_unknown() {
        assert!(ThemeColors::builtin_by_name("totally-not-a-theme").is_none());
        assert!(ThemeColors::builtin_by_name("").is_none());
    }

    #[test]
    fn builtin_names_and_listing_are_consistent() {
        let names: Vec<&str> = ThemeColors::builtin_names().iter().copied().collect();
        let listing_names: Vec<&str> = ThemeColors::builtin_listing()
            .iter()
            .map(|(n, _, _)| *n)
            .collect();
        assert_eq!(
            names, listing_names,
            "builtin_names() and builtin_listing() must be in sync"
        );
    }

    #[test]
    fn builtin_listing_has_description_and_variant() {
        for (name, desc, variant) in ThemeColors::builtin_listing() {
            assert!(!name.is_empty(), "builtin name cannot be empty");
            assert!(!desc.is_empty(), "builtin '{name}' description missing");
            assert!(
                matches!(*variant, "dark" | "light"),
                "builtin '{name}' variant must be 'dark' or 'light', got '{variant}'"
            );
        }
    }

    #[test]
    fn builtin_dark_is_resolvable_by_name() {
        let by_name = ThemeColors::builtin_by_name("dark").expect("dark must resolve");
        let direct = ThemeColors::dark();
        assert_eq!(by_name.primary, direct.primary);
        assert_eq!(by_name.bg_base, direct.bg_base);
        assert_eq!(by_name.error, direct.error);
    }

    #[test]
    fn from_theme_maps_all_token_fields() {
        // Build a ThemeTokens where every field holds a unique sentinel color.
        // After from_theme(), the corresponding ThemeColors slot must NOT be
        // the dark()-fallback — meaning the token was actually applied.
        fn hex(v: u32) -> ThemeColor {
            ThemeColor::Hex(format!("#{:06X}", v))
        }

        let mut t = ThemeTokens::default();
        // assign unique non-default hex to every token
        t.accent = hex(0x010203);
        t.border = hex(0x040506);
        t.border_accent = hex(0x070809);
        t.border_muted = hex(0x0a0b0c);
        t.success = hex(0x0d0e0f);
        t.error = hex(0x101112);
        t.warning = hex(0x131415);
        t.muted = hex(0x161718);
        t.dim = hex(0x191a1b);
        t.text = hex(0x1c1d1e);
        t.thinking_text = hex(0x1f2021);
        t.selected_bg = hex(0x222324);
        t.user_message_bg = hex(0x252627);
        t.user_message_text = hex(0x282930);
        t.custom_message_bg = hex(0x313233);
        t.custom_message_text = hex(0x343536);
        t.custom_message_label = hex(0x373839);
        t.tool_pending_bg = hex(0x3a3b3c);
        t.tool_success_bg = hex(0x3d3e3f);
        t.tool_error_bg = hex(0x404142);
        t.tool_title = hex(0x434445);
        t.tool_output = hex(0x464748);
        t.md_heading = hex(0x494a4b);
        t.md_link = hex(0x4c4d4e);
        t.md_link_url = hex(0x4f5051);
        t.md_code = hex(0x525354);
        t.md_code_block = hex(0x555657);
        t.md_code_block_border = hex(0x585960);
        t.md_quote = hex(0x616263);
        t.md_quote_border = hex(0x646566);
        t.md_hr = hex(0x676869);
        t.md_list_bullet = hex(0x6a6b6c);
        t.tool_diff_added = hex(0x6d6e6f);
        t.tool_diff_removed = hex(0x707172);
        t.tool_diff_context = hex(0x737475);
        t.syntax_comment = hex(0x767778);
        t.syntax_keyword = hex(0x797a7b);
        t.syntax_function = hex(0x7c7d7e);
        t.syntax_variable = hex(0x7f8081);
        t.syntax_string = hex(0x828384);
        t.syntax_number = hex(0x858687);
        t.syntax_type = hex(0x888990);
        t.syntax_operator = hex(0x919293);
        t.syntax_punctuation = hex(0x949596);
        t.thinking_off = hex(0x979899);
        t.thinking_minimal = hex(0x9a9b9c);
        t.thinking_low = hex(0x9d9e9f);
        t.thinking_medium = hex(0xa0a1a2);
        t.thinking_high = hex(0xa3a4a5);
        t.thinking_xhigh = hex(0xa6a7a8);
        t.bash_mode = hex(0xa9aaab);

        let theme = Theme {
            name: "test".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: t,
            source: PathBuf::new(),
        };

        let tc = ThemeColors::from_theme(&theme);
        let dark = ThemeColors::dark();

        // Every semantic slot that we mapped must differ from the dark()
        // fallback — proving the token applied.
        assert_ne!(tc.primary, dark.primary, "primary not mapped");
        assert_ne!(tc.success, dark.success, "success not mapped");
        assert_ne!(tc.error, dark.error, "error not mapped");
        assert_ne!(tc.warning, dark.warning, "warning not mapped");
        assert_ne!(tc.border_base, dark.border_base, "border_base not mapped");
        assert_ne!(
            tc.border_focus, dark.border_focus,
            "border_focus not mapped"
        );
        assert_ne!(
            tc.text_primary, dark.text_primary,
            "text_primary not mapped"
        );
        assert_ne!(tc.text_muted, dark.text_muted, "text_muted not mapped");
        assert_ne!(tc.text_dim, dark.text_dim, "text_dim not mapped");
        assert_ne!(tc.diff_added, dark.diff_added, "diff_added not mapped");
        assert_ne!(
            tc.diff_removed, dark.diff_removed,
            "diff_removed not mapped"
        );
        assert_ne!(
            tc.diff_context, dark.diff_context,
            "diff_context not mapped"
        );
        assert_ne!(tc.md_heading, dark.md_heading, "md_heading not mapped");
        assert_ne!(tc.md_link, dark.md_link, "md_link not mapped");
        assert_ne!(tc.md_link_url, dark.md_link_url, "md_link_url not mapped");
        assert_ne!(tc.md_code, dark.md_code, "md_code not mapped");
        assert_ne!(
            tc.md_code_block, dark.md_code_block,
            "md_code_block not mapped"
        );
        assert_ne!(
            tc.md_code_block_border, dark.md_code_block_border,
            "md_code_block_border not mapped"
        );
        assert_ne!(tc.md_quote, dark.md_quote, "md_quote not mapped");
        assert_ne!(
            tc.md_quote_border, dark.md_quote_border,
            "md_quote_border not mapped"
        );
        assert_ne!(tc.md_hr, dark.md_hr, "md_hr not mapped");
        assert_ne!(
            tc.md_list_bullet, dark.md_list_bullet,
            "md_list_bullet not mapped"
        );
        assert_ne!(
            tc.syntax_comment, dark.syntax_comment,
            "syntax_comment not mapped"
        );
        assert_ne!(
            tc.syntax_keyword, dark.syntax_keyword,
            "syntax_keyword not mapped"
        );
        assert_ne!(
            tc.syntax_function, dark.syntax_function,
            "syntax_function not mapped"
        );
        assert_ne!(
            tc.syntax_variable, dark.syntax_variable,
            "syntax_variable not mapped"
        );
        assert_ne!(
            tc.syntax_string, dark.syntax_string,
            "syntax_string not mapped"
        );
        assert_ne!(
            tc.syntax_number, dark.syntax_number,
            "syntax_number not mapped"
        );
        assert_ne!(tc.syntax_type, dark.syntax_type, "syntax_type not mapped");
        assert_ne!(
            tc.syntax_operator, dark.syntax_operator,
            "syntax_operator not mapped"
        );
        assert_ne!(
            tc.syntax_punctuation, dark.syntax_punctuation,
            "syntax_punctuation not mapped"
        );
        assert_ne!(
            tc.thinking_off, dark.thinking_off,
            "thinking_off not mapped"
        );
        assert_ne!(
            tc.thinking_minimal, dark.thinking_minimal,
            "thinking_minimal not mapped"
        );
        assert_ne!(
            tc.thinking_low, dark.thinking_low,
            "thinking_low not mapped"
        );
        assert_ne!(
            tc.thinking_medium, dark.thinking_medium,
            "thinking_medium not mapped"
        );
        assert_ne!(
            tc.thinking_high, dark.thinking_high,
            "thinking_high not mapped"
        );
        assert_ne!(
            tc.thinking_xhigh, dark.thinking_xhigh,
            "thinking_xhigh not mapped"
        );
        assert_ne!(tc.bash_mode, dark.bash_mode, "bash_mode not mapped");
    }

    #[test]
    fn discover_themes_with_builtins_adds_all_builtins_on_empty_dirs() {
        let cwd = make_dir();
        let agent_dir = make_dir();

        let themes = discover_themes_with_builtins(cwd.path(), agent_dir.path());
        let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();

        for expected in ThemeColors::builtin_names() {
            assert!(
                names.contains(expected),
                "builtin '{expected}' must appear in discover_themes_with_builtins; got {names:?}"
            );
        }
    }

    #[test]
    fn discover_themes_with_builtins_respects_ondisk_shadowing() {
        let cwd = make_dir();
        let agent_dir = make_dir();

        // Write a custom on-disk "dark.json" that shadows the builtin
        let themes_dir = cwd.path().join(".cade").join("themes");
        fs::create_dir_all(&themes_dir).unwrap();
        let custom_theme = Theme {
            name: "dark".to_string(),
            description: Some("Custom override".to_string()),
            author: Some("user".to_string()),
            variant: Some("dark".to_string()),
            vars: Default::default(),
            colors: ThemeTokens::default(),
            source: PathBuf::new(),
        };
        fs::write(
            themes_dir.join("dark.json"),
            serde_json::to_string(&custom_theme).unwrap(),
        )
        .unwrap();

        let themes = discover_themes_with_builtins(cwd.path(), agent_dir.path());
        let dark_entries: Vec<&Theme> = themes.iter().filter(|t| t.name == "dark").collect();

        assert_eq!(
            dark_entries.len(),
            1,
            "on-disk 'dark' must shadow the builtin — only one entry expected"
        );
        assert_eq!(
            dark_entries[0].description.as_deref(),
            Some("Custom override"),
            "on-disk theme must win over builtin listing"
        );
    }

    #[test]
    fn light_theme_text_colors_are_readable_on_white() {
        // Regression guard for Phase 7: the GUI maps ColorDef::Reset to a
        // pastel light color (205, 214, 244) which is invisible on the
        // light theme's near-white background.  light() must use explicit
        // dark RGB values for any text slot that will render on
        // bg_base / bg_surface*.
        let light = ThemeColors::light();

        fn is_readably_dark(c: &ColorDef) -> bool {
            match c {
                ColorDef::Rgb(r, g, b) => {
                    // YIQ perceived brightness; <128 ~= dark enough to read on white.
                    let brightness = (*r as u32 * 299 + *g as u32 * 587 + *b as u32 * 114) / 1000;
                    brightness < 180
                }
                ColorDef::Reset => false, // Reset = terminal default, not safe on GUI
            }
        }

        assert!(
            is_readably_dark(&light.text_primary),
            "light.text_primary must be dark enough to read on white; got {:?}",
            light.text_primary
        );
        assert!(
            is_readably_dark(&light.md_code_block),
            "light.md_code_block must be dark enough to read on white; got {:?}",
            light.md_code_block
        );
    }

    #[test]
    fn test_load_theme_valid() {
        // -- Setup & Fixtures
        let dir = make_dir();
        let path = dir.path().join("mytheme.json");
        // A valid theme must have all required token fields.
        // We use serde_json::to_string of a Theme with default colors.
        let theme = Theme {
            name: "mytheme".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: ThemeTokens::default(),
            source: PathBuf::new(),
        };
        let json_str = serde_json::to_string(&theme).unwrap();
        fs::write(&path, json_str).unwrap();

        // -- Exec
        let loaded = load_theme(&path).unwrap();

        // -- Check
        assert_eq!(loaded.name, "mytheme");
        assert_eq!(loaded.source, path);
    }

    #[test]
    fn test_theme_color_hex_serde() {
        // -- Setup & Fixtures
        let color = ThemeColor::Hex("#ff0000".to_string());

        // -- Exec
        let json = serde_json::to_string(&color).unwrap();
        let back: ThemeColor = serde_json::from_str(&json).unwrap();

        // -- Check
        assert_eq!(back, color);
    }

    #[test]
    fn test_theme_color_index_serde() {
        // -- Setup & Fixtures
        let color = ThemeColor::Index(42);

        // -- Exec
        let json = serde_json::to_string(&color).unwrap();
        let back: ThemeColor = serde_json::from_str(&json).unwrap();

        // -- Check
        assert_eq!(back, color);
    }

    #[test]
    fn test_load_tmtheme_valid() {
        let dir = make_dir();
        let path = dir.path().join("mytheme.tmTheme");

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>mytheme</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#1E1E1E</string>
                <key>foreground</key>
                <string>#D4D4D4</string>
                <key>selection</key>
                <string>#264F78</string>
                <key>lineHighlight</key>
                <string>#2A2D2E</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword.control</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#C586C0</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>"#;
        fs::write(&path, xml).unwrap();

        let loaded = load_theme(&path).unwrap();
        assert_eq!(loaded.name, "mytheme");
        assert_eq!(loaded.source, path);
        // The parser should extract #1E1E1E for userMessageBg (from background)
        assert_eq!(
            loaded.colors.user_message_bg,
            ThemeColor::Hex("#1E1E1E".to_string())
        );
        // The parser should extract #D4D4D4 for text (from foreground)
        assert_eq!(loaded.colors.text, ThemeColor::Hex("#D4D4D4".to_string()));
        // The parser should extract #C586C0 for accent (from keyword.control)
        assert_eq!(loaded.colors.accent, ThemeColor::Hex("#C586C0".to_string()));
    }

    // -- Step 2: metadata fields
    #[test]
    fn test_theme_default_variant_is_none() {
        let t = Theme {
            name: "x".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: ThemeTokens::default(),
            source: PathBuf::new(),
        };
        assert!(t.variant.is_none());
        assert!(t.description.is_none());
        assert!(t.author.is_none());
    }

    #[test]
    fn test_theme_metadata_round_trips_json() {
        let dir = make_dir();
        let path = dir.path().join("meta.json");
        let json = r#"{
            "name": "meta-theme",
            "description": "A test theme",
            "author": "CADE",
            "variant": "dark",
            "colors": {}
        }"#;
        fs::write(&path, json).unwrap();
        let loaded = load_theme(&path).unwrap();
        assert_eq!(loaded.description.as_deref(), Some("A test theme"));
        assert_eq!(loaded.author.as_deref(), Some("CADE"));
        assert_eq!(loaded.variant.as_deref(), Some("dark"));
    }

    #[test]
    fn test_theme_missing_metadata_defaults_none() {
        let dir = make_dir();
        let path = dir.path().join("bare.json");
        let json = r#"{"name": "bare", "colors": {}}"#;
        fs::write(&path, json).unwrap();
        let loaded = load_theme(&path).unwrap();
        assert!(loaded.description.is_none());
        assert!(loaded.author.is_none());
        assert!(loaded.variant.is_none());
    }
    #[test]
    fn brighten_color_clamps_at_255() {
        let c = super::brighten_color(ColorDef::Rgb(250, 240, 200), 30);
        assert_eq!(c, ColorDef::Rgb(255, 255, 230));
    }

    #[test]
    fn brighten_color_zero_is_identity() {
        let c = super::brighten_color(ColorDef::Rgb(100, 150, 200), 0);
        assert_eq!(c, ColorDef::Rgb(100, 150, 200));
    }

    #[test]
    fn brighten_color_reset_passthrough() {
        let c = super::brighten_color(ColorDef::Reset, 50);
        assert_eq!(c, ColorDef::Reset);
    }

    #[test]
    fn dim_color_clamps_at_zero() {
        let c = super::dim_color(ColorDef::Rgb(10, 5, 0), 30);
        assert_eq!(c, ColorDef::Rgb(0, 0, 0));
    }

    #[test]
    fn dim_color_zero_is_identity() {
        let c = super::dim_color(ColorDef::Rgb(100, 150, 200), 0);
        assert_eq!(c, ColorDef::Rgb(100, 150, 200));
    }

    #[test]
    fn dim_color_reset_passthrough() {
        let c = super::dim_color(ColorDef::Reset, 50);
        assert_eq!(c, ColorDef::Reset);
    }

    #[test]
    fn from_theme_derives_spinner_from_primary() {
        // A theme with no spinnerAccent should derive spinner_* from primary.
        let mut t = Theme {
            name: "test-spinner".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        t.colors.accent = ThemeColor::Hex("#80a0ff".to_string());
        let tc = ThemeColors::from_theme(&t);
        // spinner_0 should match primary (resolved from accent)
        assert_eq!(tc.spinner_0, tc.primary);
        // spinner_1 should be brighter than spinner_0
        assert_ne!(tc.spinner_1, tc.spinner_0);
    }

    #[test]
    fn from_theme_fallback_thinking_xhigh_to_error() {
        // thinkingXhigh defaults to empty string → Reset → should fall back to error.
        let mut t = Theme {
            name: "test-fallback".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        // Set error so the fallback has something real to use.
        t.colors.error = ThemeColor::Hex("#ff5555".to_string());
        let tc = ThemeColors::from_theme(&t);
        // thinkingXhigh should have fallen back to error, not remain Reset
        assert_eq!(tc.thinking_xhigh, ColorDef::Rgb(255, 85, 85));
    }

    #[test]
    fn from_theme_fallback_bash_mode_to_warning() {
        let mut t = Theme {
            name: "test-fallback".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        // Set warning so the fallback has something real to use.
        t.colors.warning = ThemeColor::Hex("#e0af68".to_string());
        let tc = ThemeColors::from_theme(&t);
        // bashMode should have fallen back to warning, not remain Reset
        assert_eq!(tc.bash_mode, ColorDef::Rgb(224, 175, 104));
    }

    #[test]
    fn from_theme_ctx_bar_derived_not_dark_default() {
        // Context-bar tokens should be derived from the theme palette,
        // not stuck on dark() defaults.
        let mut t = Theme {
            name: "test-ctx".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        t.colors.accent = ThemeColor::Hex("#ff0000".to_string());
        let tc = ThemeColors::from_theme(&t);
        let dark = ThemeColors::dark();
        // ctx_bar_native_tools derives from primary, which came from accent
        // — it should NOT equal the dark() built-in value
        assert_ne!(tc.ctx_bar_native_tools, dark.ctx_bar_native_tools);
    }
}

// endregion: --- Tests

// region:    --- Support

fn resolve_color(c: &ThemeColor, vars: &std::collections::HashMap<String, ThemeColor>) -> ColorDef {
    use ThemeColor;
    match c {
        ThemeColor::Index(_i) => ColorDef::Reset, // ColorDef doesn't support index natively
        ThemeColor::Hex(s) if s.is_empty() => ColorDef::Reset,
        ThemeColor::Hex(s) => {
            // Might be a variable reference
            if let Some(var_val) = vars.get(s) {
                return resolve_color(var_val, vars);
            }
            // Try to parse as hex
            if let Some(rgb) = parse_hex(s) {
                return ColorDef::Rgb(rgb.0, rgb.1, rgb.2);
            }
            ColorDef::Reset
        }
    }
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Brighten an RGB color by a fixed amount, clamping at 255.
fn brighten_color(c: ColorDef, amount: u8) -> ColorDef {
    match c {
        ColorDef::Rgb(r, g, b) => ColorDef::Rgb(
            r.saturating_add(amount),
            g.saturating_add(amount),
            b.saturating_add(amount),
        ),
        ColorDef::Reset => ColorDef::Reset,
    }
}

/// Dim (darken) an RGB color by a fixed amount, clamping at 0.
fn dim_color(c: ColorDef, amount: u8) -> ColorDef {
    match c {
        ColorDef::Rgb(r, g, b) => ColorDef::Rgb(
            r.saturating_sub(amount),
            g.saturating_sub(amount),
            b.saturating_sub(amount),
        ),
        ColorDef::Reset => ColorDef::Reset,
    }
}

// endregion: --- Support

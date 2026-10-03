//! Semantic Design Tokens for CADE GUI.
//!
//! Enforces a unified, precision-engineered dark workstation aesthetic based on
//! `baseline-ui` and `frontend-design` standards, eliminating arbitrary hex sprawl.

/// Core surface backgrounds
pub mod surface {
    pub const CANVAS: &str = "bg-[#040711]";
    pub const PANEL: &str = "bg-[#090d16]";
    pub const CARD: &str = "bg-slate-900/60 border border-slate-800/80 rounded-xl";
    pub const CARD_HOVER: &str = "hover:border-slate-700/80 hover:bg-slate-900/80 transition-all duration-150";
    pub const HEADER: &str = "border-b border-slate-800/80 bg-[#090d16]/90 backdrop-blur-md";
    pub const INSET: &str = "bg-slate-950/60 border border-slate-800/60 rounded-lg";
}

/// Border tokens
pub mod border {
    pub const SUBTLE: &str = "border-slate-800/80";
    pub const MUTED: &str = "border-slate-700/60";
    pub const ACCENT_SKY: &str = "border-sky-500/50";
    pub const ACCENT_EMERALD: &str = "border-emerald-500/50";
    pub const ACCENT_AMBER: &str = "border-amber-500/50";
}

/// Typography tokens with balance and tabular numbers
pub mod typography {
    pub const TITLE_PAGE: &str = "text-lg font-semibold text-slate-100 tracking-tight select-none";
    pub const HEADING_SECTION: &str = "text-sm font-semibold text-slate-200 tracking-tight";
    pub const BODY_MUTED: &str = "text-xs text-slate-400 leading-relaxed";
    pub const LABEL_CAPS: &str = "text-[10px] font-mono font-bold uppercase tracking-wider";
    pub const CODE_INLINE: &str = "font-mono text-xs text-sky-300 bg-slate-950 px-1.5 py-0.5 rounded border border-slate-800";
    pub const TABULAR_NUM: &str = "font-mono tabular-nums tracking-tight";
}

/// Interaction and accessibility tokens
pub mod interaction {
    pub const FOCUS_RING: &str = "focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:outline-none";
    pub const BUTTON_PILL: &str = "px-3 py-1 text-xs font-medium rounded-lg cursor-pointer transition duration-150 select-none";
    pub const CLICKABLE_CARD: &str = "cursor-pointer transition-all duration-200 focus-visible:ring-2 focus-visible:ring-sky-400 outline-none";
}

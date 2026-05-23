//! Overlay render functions — one sub-module per overlay panel.

pub mod agents;
pub mod artifacts;
pub mod checkpoints;
pub mod context;
pub mod mcp;
pub mod memory;
pub mod menu;
pub mod models;
pub mod palette;
pub mod profiles;
pub mod settings;
pub mod skills;
pub mod stats;
pub mod tools;

pub use agents::render_agents_overlay;
pub use artifacts::render_artifacts_overlay;
pub use checkpoints::render_checkpoints_overlay;
pub use context::render_context_overlay;
pub use mcp::render_mcp_overlay;
pub use memory::render as render_memory_overlay;
pub use menu::render_menu_overlay;
pub use models::render_model_picker;
pub use palette::render_palette_overlay;
pub use profiles::render_profiles_overlay;
pub use stats::render_stats_overlay;
pub use tools::{render_question_widget, render_tools_overlay};

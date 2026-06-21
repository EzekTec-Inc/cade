#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::too_many_arguments)]
// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod agent;
pub mod backends;
pub mod mcp;
pub mod moa;
pub mod plugins;
pub mod routing;
pub mod subagents;
pub mod team;
pub mod tools;

// endregion: --- Modules

#[cfg(test)]
mod routing_test;

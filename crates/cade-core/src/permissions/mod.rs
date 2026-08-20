// region:    --- Modules

// endregion: --- Modules

pub mod authority;
pub mod constitution;
pub mod checks;
pub mod manager;
pub mod rules;
pub mod service;

pub use authority::*;
pub use constitution::*;
pub use checks::*;
pub use manager::*;
pub use rules::*;
pub use service::*;

#[cfg(test)]
mod tests;

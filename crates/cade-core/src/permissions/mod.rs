// region:    --- Modules

// endregion: --- Modules

pub mod authority;
pub mod checks;
pub mod constitution;
pub mod manager;
pub mod rules;
pub mod service;

pub use authority::*;
pub use checks::*;
pub use constitution::*;
pub use manager::*;
pub use rules::*;
pub use service::*;

#[cfg(test)]
mod tests;

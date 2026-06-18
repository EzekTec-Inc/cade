use derive_more::{Display, From};

/// Crate-level result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Unified error type for the cade-agent crate.
///
/// Wraps common external errors and supports ad-hoc string messages via
/// [`Error::custom`] or [`Error::custom_from_err`].
#[derive(Debug, Display, From)]
#[display("{_0}")]
pub enum Error {
    #[from(String, &String, &str)]
    Custom(String),

    // -- Externals
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
    #[from]
    Reqwest(reqwest::Error),
    #[from]
    #[cfg(feature = "desktop")]
    Desktop(cade_desktop::Error),
}

// region:    --- Custom

impl Error {
    pub fn custom_from_err(err: impl std::error::Error) -> Self {
        Self::Custom(err.to_string())
    }

    pub fn custom(val: impl Into<String>) -> Self {
        Self::Custom(val.into())
    }
}

// endregion: --- Custom

// region:    --- Error Boilerplate

impl std::error::Error for Error {}

// endregion: --- Error Boilerplate

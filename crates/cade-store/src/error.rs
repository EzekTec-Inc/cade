use derive_more::{Display, From};

/// Crate-level result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Unified error type for the cade-store crate.
///
/// Wraps database (`rusqlite`, `r2d2`), I/O, crypto, serialization, and
/// delegation errors from `cade-core` and `cade-ai`.
#[derive(Debug, Display, From)]
#[display("{_0}")]
pub enum Error {
    #[from(String, &String, &str)]
    Custom(String),

    // -- Externals
    #[from]
    Sqlite(rusqlite::Error),
    #[from]
    R2d2(r2d2::Error),
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
    #[from]
    Crypto(aes_gcm::Error),
    #[from]
    Core(cade_core::Error),
    #[from]
    Ai(cade_ai::Error),
    #[from]
    AddrParse(std::net::AddrParseError),
    #[from]
    Base64(base64::DecodeError),
    #[from]
    FromUtf8(std::string::FromUtf8Error),
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

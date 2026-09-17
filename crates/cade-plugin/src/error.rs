use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
    #[display("custom error: {_0}")]
    Custom(String),

    #[display("integrity check failed: expected sha256 {expected}, got {actual}")]
    IntegrityError { expected: String, actual: String },

    // -- Externals
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
    #[from]
    Toml(toml::de::Error),
}

impl Error {
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}

impl std::error::Error for Error {}

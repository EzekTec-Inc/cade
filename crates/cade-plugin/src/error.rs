use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
    #[display("custom error: {_0}")]
    Custom(String),

    // -- Externals
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
}

impl Error {
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}

impl std::error::Error for Error {}

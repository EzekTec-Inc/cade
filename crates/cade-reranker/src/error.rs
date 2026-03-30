use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
    #[display("reranker error: {_0}")]
    Custom(String),
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
    #[from]
    Reqwest(reqwest::Error),
}

impl Error {
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}

impl std::error::Error for Error {}

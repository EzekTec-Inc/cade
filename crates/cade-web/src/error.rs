use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
    #[display("web error: {_0}")]
    Custom(String),
    #[from]
    Reqwest(reqwest::Error),
    #[from]
    Io(std::io::Error),
    #[from]
    Url(url::ParseError),
}

impl Error {
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}

impl std::error::Error for Error {}

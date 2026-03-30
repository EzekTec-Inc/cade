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

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_custom_creation() {
        let err = Error::custom("test error");
        assert!(matches!(err, Error::Custom(_)));
        assert_eq!(err.to_string(), "reranker error: test error");
    }

    #[test]
    fn error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn error_from_serde_json() {
        let json_err = serde_json::from_str::<i32>("invalid").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::SerdeJson(_)));
    }

    #[test]
    fn error_display_formatting() {
        let err = Error::custom("test message");
        assert_eq!(format!("{err}"), "reranker error: test message");
    }

    #[test]
    fn error_implements_std_error() {
        let err = Error::custom("test");
        // Error trait is implemented — this compiles and doesn't panic.
        let _: &dyn std::error::Error = &err;
    }
}

// endregion: --- Tests

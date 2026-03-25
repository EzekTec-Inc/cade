use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use derive_more::{Display, From};
use serde_json::json;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
#[display("{self:?}")]
pub enum Error {
    #[from(String, &String, &str)]
    Custom(String),

    // -- Externals
    #[from]
    Sqlite(rusqlite::Error),
    #[from]
    Io(std::io::Error),
    #[from]
    SerdeJson(serde_json::Error),
    #[from]
    Crypto(aes_gcm::Error),
    #[from]
    Axum(axum::Error),
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

// region:    --- Axum Response

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            Error::Sqlite(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {err}"),
            ),
            Error::Io(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {err}"),
            ),
            Error::SerdeJson(err) => (
                StatusCode::BAD_REQUEST,
                format!("JSON serialization error: {err}"),
            ),
            Error::Crypto(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Encryption error: {err}"),
            ),
            Error::Axum(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Server error: {err}"),
            ),
            Error::Core(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Core error: {err}"),
            ),
            Error::Ai(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("AI provider error: {err}"),
            ),
            Error::AddrParse(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Address parse error: {err}"),
            ),
            Error::Base64(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Base64 error: {err}"),
            ),
            Error::FromUtf8(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("UTF-8 error: {err}"),
            ),
            Error::Custom(msg) => (StatusCode::BAD_REQUEST, msg),
        };

        let body = Json(json!({ "error": error_message }));
        (status, body).into_response()
    }
}

// endregion: --- Axum Response

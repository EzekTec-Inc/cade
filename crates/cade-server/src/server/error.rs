use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub type Result<T> = core::result::Result<T, Error>;

/// Server-level error that wraps [`cade_store::error::Error`] and adds
/// HTTP response conversion for Axum handlers.
#[derive(Debug)]
pub enum Error {
    Store(cade_store::error::Error),
    Custom(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Store(e) => write!(f, "{e}"),
            Error::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<cade_store::error::Error> for Error {
    fn from(e: cade_store::error::Error) -> Self {
        Error::Store(e)
    }
}

impl From<std::net::AddrParseError> for Error {
    fn from(e: std::net::AddrParseError) -> Self {
        Error::Store(cade_store::error::Error::AddrParse(e))
    }
}

impl Error {
    pub fn custom(val: impl Into<String>) -> Self {
        Self::Custom(val.into())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use cade_store::error::Error as StoreError;

        let (status, error_message) = match self {
            Error::Store(ref inner) => match inner {
                StoreError::Sqlite(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error: {err}"),
                ),
                StoreError::Io(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("IO error: {err}"),
                ),
                StoreError::SerdeJson(err) => (
                    StatusCode::BAD_REQUEST,
                    format!("JSON serialization error: {err}"),
                ),
                StoreError::Crypto(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Encryption error: {err}"),
                ),
                StoreError::Core(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Core error: {err}"),
                ),
                StoreError::Ai(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("AI provider error: {err}"),
                ),
                StoreError::AddrParse(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Address parse error: {err}"),
                ),
                StoreError::Base64(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Base64 error: {err}"),
                ),
                StoreError::FromUtf8(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("UTF-8 error: {err}"),
                ),
                StoreError::Custom(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            },
            Error::Custom(msg) => (StatusCode::BAD_REQUEST, msg),
        };

        let body = Json(json!({ "error": error_message }));
        (status, body).into_response()
    }
}

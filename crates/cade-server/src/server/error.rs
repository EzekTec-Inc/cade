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

// region:    --- P3-1: Generic 5xx responses

/// Build a 5xx response body that does not leak internal detail.
///
/// The full `detail` is sent to `tracing::error!` along with the
/// generated `request_id` so operators can map a client-reported id
/// back to the real error in logs.  The client only sees:
///
///   `{"error": "internal error", "request_id": "<uuid>"}`
pub(crate) fn internal_error_response(detail: &str) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    tracing::error!(request_id = %request_id, detail = %detail, "500 Internal Server Error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "internal error", "request_id": request_id })),
    )
        .into_response()
}

// endregion: --- P3-1: Generic 5xx responses

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use cade_store::error::Error as StoreError;

        // Bucket every variant as either server-side (5xx → generic
        // body) or client-side (4xx → echo the already-safe message).
        match self {
            Error::Store(ref inner) => match inner {
                // 4xx — user-triggered, message is safe by construction.
                StoreError::SerdeJson(err) => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("JSON serialization error: {err}") })),
                )
                    .into_response(),
                StoreError::Custom(msg) => {
                    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
                }

                // 5xx — internal, generic body + log correlation id.
                StoreError::Sqlite(err) => internal_error_response(&format!("sqlite: {err}")),
                StoreError::Io(err) => internal_error_response(&format!("io: {err}")),
                StoreError::Crypto(err) => internal_error_response(&format!("crypto: {err}")),
                StoreError::Core(err) => internal_error_response(&format!("core: {err}")),
                StoreError::Ai(err) => internal_error_response(&format!("ai: {err}")),
                StoreError::AddrParse(err) => {
                    internal_error_response(&format!("addr_parse: {err}"))
                }
                StoreError::Base64(err) => internal_error_response(&format!("base64: {err}")),
                StoreError::FromUtf8(err) => internal_error_response(&format!("from_utf8: {err}")),
            },
            Error::Custom(msg) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
            }
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_test;

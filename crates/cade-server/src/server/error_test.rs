//! P3-1: generic 5xx error responses.
//!
//! These tests lock in the contract:
//!   * Any 5xx response emits `{"error": "internal error", "request_id": <uuid>}`
//!     and never leaks internal detail (file paths, SQL text, stack info).
//!   * 4xx responses keep their existing shape so CLI/client consumers
//!     that already parse `error` / `detail` strings continue to work.
//!   * Every 5xx response gets a fresh, unique `request_id`.

use super::Error;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use cade_store::error::Error as StoreError;
use serde_json::Value;

async fn body_json(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    (status, v)
}

// -- 5xx generic body

#[tokio::test]
async fn internal_error_returns_generic_body_without_leak() {
    // A StoreError::Io carrying a path-looking string that MUST NOT be
    // echoed back to the client.
    let leaky = "/etc/passwd not readable: secret_sauce";
    let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, leaky);
    let err = Error::Store(StoreError::Io(io));

    let (status, body) = body_json(err.into_response()).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal error");
    assert!(
        !body.to_string().contains(leaky),
        "5xx response leaked internal detail: {body}"
    );
    assert!(
        !body.to_string().contains("IO error"),
        "5xx response leaked variant label: {body}"
    );
    // request_id is present and non-empty
    let rid = body["request_id"].as_str().expect("request_id is a string");
    assert!(!rid.is_empty(), "request_id must be non-empty");
}

#[tokio::test]
async fn sqlite_error_does_not_leak_query_text() {
    let err = Error::Store(StoreError::Sqlite(rusqlite::Error::InvalidQuery));
    let (status, body) = body_json(err.into_response()).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal error");
    assert!(
        !body.to_string().to_lowercase().contains("invalidquery"),
        "sqlite variant label leaked: {body}"
    );
    assert!(body["request_id"].is_string());
}

#[tokio::test]
async fn each_5xx_response_has_unique_request_id() {
    let mk = || {
        let io = std::io::Error::other("inner");
        Error::Store(StoreError::Io(io))
    };

    let (_, b1) = body_json(mk().into_response()).await;
    let (_, b2) = body_json(mk().into_response()).await;

    let r1 = b1["request_id"].as_str().unwrap();
    let r2 = b2["request_id"].as_str().unwrap();
    assert_ne!(r1, r2, "request_id must be unique per response");
}

// -- 4xx shape unchanged

#[tokio::test]
async fn custom_error_preserves_400_message() {
    // This message IS user-safe by construction (set by handler code),
    // so it must flow through untouched.
    let err = Error::custom("invalid conversation id");
    let (status, body) = body_json(err.into_response()).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid conversation id");
    assert!(
        body.get("request_id").is_none(),
        "4xx responses must NOT include request_id: {body}"
    );
}

#[tokio::test]
async fn store_custom_preserves_400_message() {
    let err = Error::Store(StoreError::Custom("not found".into()));
    let (status, body) = body_json(err.into_response()).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "not found");
}

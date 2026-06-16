//! P3-1: `server_err()` also emits the generic 5xx shape.

use super::server_err;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

async fn body_json(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    (status, v)
}

#[tokio::test]
async fn server_err_emits_generic_body() {
    let leaky = "sql error at column 3: SELECT * FROM users WHERE email='abc'";
    let tuple = server_err(leaky.to_string());

    let (status, body) = body_json(tuple.into_response()).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal error");
    assert!(
        !body.to_string().contains("SELECT"),
        "server_err leaked SQL text: {body}"
    );
    assert!(
        !body.to_string().contains("email"),
        "server_err leaked column name: {body}"
    );
    assert!(
        body.get("detail").is_none(),
        "old `detail` field must be gone in 5xx responses: {body}"
    );
    assert!(!body["request_id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn server_err_request_ids_are_unique() {
    let a = server_err("a".into()).into_response();
    let b = server_err("b".into()).into_response();
    let (_, ba) = body_json(a).await;
    let (_, bb) = body_json(b).await;
    assert_ne!(ba["request_id"], bb["request_id"]);
}

use crate::server::state::AppState;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct StreamRequest {
    pub url: String,
}

pub async fn stream_http_handler(
    State(_state): State<AppState>,
    Query(params): Query<StreamRequest>,
) -> Response {
    let client = Client::new();

    let req = match client.get(&params.url).send().await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Failed to connect to upstream: {}", e),
            )
                .into_response();
        }
    };

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let stream = req
        .bytes_stream()
        .map(|result| result.map_err(std::io::Error::other));

    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .unwrap_or_else(|e| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("failed to build response: {e}")))
                .unwrap()
        })
}

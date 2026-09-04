use crate::server::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

pub async fn list_approvals(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _conn = state
        .db
        .get()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let pending = cade_store::sqlite::list_pending_approvals(&state.db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "approvals": pending })))
}

#[derive(serde::Deserialize)]
pub struct ActionPayload {
    pub action: String, // "approve" or "deny"
    pub feedback: Option<String>,
}

pub async fn action_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ActionPayload>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = match payload.action.as_str() {
        "approve" => "approved".to_string(),
        "deny" => {
            if let Some(fb) = &payload.feedback {
                if !fb.trim().is_empty() {
                    format!("denied:{}", fb.trim())
                } else {
                    "denied".to_string()
                }
            } else {
                "denied".to_string()
            }
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid action. Must be 'approve' or 'deny'.".to_string(),
            ));
        }
    };

    let _conn = state
        .db
        .get()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Check if the approval exists first
    let current_status = cade_store::sqlite::get_approval_status(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if current_status.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Approval request '{}' not found.", id),
        ));
    }

    cade_store::sqlite::set_approval_status(&state.db, &id, &status)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "id": id, "status": status })))
}

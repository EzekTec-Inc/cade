/// Code intelligence API handlers.
///
/// These endpoints wrap the `cade-codeintel` crate.  The symbol index lives in
/// the same SQLite database as the rest of CADE state.
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::state::AppState;

// region:    --- Query params

#[derive(Deserialize)]
pub struct SymbolSearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub repo_root: Option<String>,
}

#[derive(Deserialize)]
pub struct RepoMapParams {
    pub max_symbols: Option<usize>,
    pub repo_root: Option<String>,
}

#[derive(Deserialize)]
pub struct RefParams {
    pub repo_root: Option<String>,
}

#[derive(Deserialize)]
pub struct DefParams {
    pub from_file: Option<String>,
}

// endregion: --- Query params

// region:    --- Handlers

/// GET /v1/symbols?q=<query>&limit=<n>&repo_root=<path>
pub async fn symbol_search(
    State(state): State<AppState>,
    Query(params): Query<SymbolSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let query = params.q.as_deref().unwrap_or("").trim().to_string();
    let limit = params.limit.unwrap_or(20);
    let _repo_root = params.repo_root.as_deref().unwrap_or(".");

    if query.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "q is required" })),
        ));
    }

    // Use cade-codeintel via shared SQLite db
    use cade_codeintel::symbol_search as do_search;
    match do_search(&state.db, &query, limit) {
        Ok(symbols) => Ok(Json(json!(symbols))),
        Err(e) => {
            // Symbol table may not exist yet (no indexing run)
            tracing::debug!("symbol_search: {e}");
            Ok(Json(json!([])))
        }
    }
}

/// GET /v1/symbols/:name/definition?from_file=<path>
pub async fn goto_definition(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<DefParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use cade_codeintel::goto_definition as do_goto;
    match do_goto(&state.db, &name, params.from_file.as_deref()) {
        Ok(Some(sym)) => Ok(Json(json!(sym))),
        Ok(None) => Ok(Json(json!(null))),
        Err(e) => {
            tracing::debug!("goto_definition: {e}");
            Ok(Json(json!(null)))
        }
    }
}

/// GET /v1/symbols/:name/refs?repo_root=<path>
pub async fn find_references(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<RefParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use cade_codeintel::find_references as do_refs;
    let repo_root = params.repo_root.as_deref().unwrap_or(".");
    match do_refs(&state.db, &name, repo_root) {
        Ok(refs) => Ok(Json(json!(refs))),
        Err(e) => {
            tracing::debug!("find_references: {e}");
            Ok(Json(json!([])))
        }
    }
}

/// GET /v1/repo-map?max_symbols=<n>&repo_root=<path>
pub async fn get_repo_map(
    State(state): State<AppState>,
    Query(params): Query<RepoMapParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use cade_codeintel::generate_repo_map;
    let max_symbols = params.max_symbols.unwrap_or(8);
    let repo_root = params.repo_root.as_deref().unwrap_or(".");
    let repo_path = std::path::Path::new(repo_root);
    match generate_repo_map(repo_path, &state.db, max_symbols) {
        Ok(map) => Ok(Json(json!({ "map": map }))),
        Err(e) => {
            tracing::debug!("get_repo_map: {e}");
            Ok(Json(json!({ "map": "(not indexed)" })))
        }
    }
}

/// POST /v1/agents/:id/index  { "repo_root": "..." }
pub async fn index_repository(
    State(state): State<AppState>,
    Path(_agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo_root = body["repo_root"].as_str().unwrap_or(".").to_string();
    let repo_path = std::path::PathBuf::from(&repo_root);

    // Ensure codeintel schema exists
    use cade_codeintel::ensure_schema;
    if let Err(e) = ensure_schema(&state.db) {
        tracing::warn!("ensure_schema: {e}");
    }

    // Spawn indexing as a background task (can be slow for large repos)
    let db = state.db.clone();
    tokio::spawn(async move {
        match cade_codeintel::index_repository(&repo_path, &db).await {
            Ok(stats) => tracing::info!(
                "index_repository: {} files, {} symbols in {}ms",
                stats.files_indexed,
                stats.symbols_added,
                stats.duration_ms
            ),
            Err(e) => tracing::warn!("index_repository failed: {e}"),
        }
    });

    Ok(Json(json!({
        "status": "indexing_started",
        "repo_root": repo_root,
        "message": "Indexing running in background. Use symbol_search after it completes."
    })))
}

// endregion: --- Handlers

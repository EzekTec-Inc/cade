//! Knowledge graph triples API endpoints (ADR-0002).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;

use crate::server::state::AppState;
use cade_store::sqlite::knowledge::{
    KnowledgeEdge, delete_knowledge_edge, insert_knowledge_edge, list_knowledge_edges,
};

#[derive(Debug, Deserialize)]
pub struct ListEdgesQuery {
    pub entity: Option<String>,
    pub relation: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEdgeRequest {
    pub entity: String,
    pub relation: String,
    pub target: String,
}

/// Seed standard architecture grounding triples if the graph is currently empty.
fn maybe_seed_default_triples(db: &cade_store::Db) {
    if let Ok(existing) = list_knowledge_edges(db, None, None)
        && existing.is_empty()
    {
        let defaults = [
            (
                "CADE",
                "implements",
                "CapabilityMesh (Native + MCP + Skills)",
            ),
            (
                "EmbeddedSession",
                "links_to",
                "SQLite & LlmRouter in-process",
            ),
            (
                "Sleeptime",
                "consolidates_at",
                "70% Context Window Threshold",
            ),
            ("TokenHeatmap", "allocates", "History vs Tool Reserve"),
            (
                "CapabilityMesh",
                "satisfies",
                "ADR-0020 Unified Execution Seam",
            ),
            ("SubagentRunner", "sandboxes_in", "Isolated Git Worktrees"),
            (
                "HeadroomProxy",
                "optimizes",
                "Prompt Caching & Token Reduction",
            ),
            (
                "KnowledgeEngine",
                "federates",
                "FTS5 BM25 + Vector Cosine Hybrid Recall",
            ),
        ];

        for (ent, rel, tgt) in defaults {
            let _ = insert_knowledge_edge(db, ent, rel, tgt, None);
        }
    }
}

/// GET /v1/knowledge/edges
///
/// Returns a list of structured knowledge edges (triples), optionally filtered by entity or relation.
pub async fn list_edges(
    State(state): State<AppState>,
    Query(query): Query<ListEdgesQuery>,
) -> Result<Json<Vec<KnowledgeEdge>>, (StatusCode, Json<serde_json::Value>)> {
    // Ensure default architecture grounding triples exist on initial query
    maybe_seed_default_triples(&state.db);

    let edges = list_knowledge_edges(
        &state.db,
        query.entity.as_deref(),
        query.relation.as_deref(),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error listing edges: {e}") })),
        )
    })?;

    Ok(Json(edges))
}

/// POST /v1/knowledge/edges
///
/// Inserts a new semantic knowledge edge (entity ➔ relation ➔ target).
pub async fn create_edge(
    State(state): State<AppState>,
    Json(payload): Json<CreateEdgeRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let ent = payload.entity.trim();
    let rel = payload.relation.trim();
    let tgt = payload.target.trim();

    if ent.is_empty() || rel.is_empty() || tgt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "entity, relation, and target must all be non-empty" })),
        ));
    }

    insert_knowledge_edge(&state.db, ent, rel, tgt, None).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to insert knowledge edge: {e}") })),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "status": "created", "entity": ent, "relation": rel, "target": tgt })),
    ))
}

/// DELETE /v1/knowledge/edges/{id}
///
/// Deletes a knowledge edge by numeric ID.
pub async fn delete_edge(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let deleted = delete_knowledge_edge(&state.db, id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to delete knowledge edge: {e}") })),
        )
    })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Knowledge edge with id {id} not found") })),
        ))
    }
}

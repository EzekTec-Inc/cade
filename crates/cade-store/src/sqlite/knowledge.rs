use crate::error::Result;
use crate::sqlite::Db;
use crate::sqlite::embedding::{Embedder, decode_embedding_blob};
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeEdge {
    pub id: i64,
    pub entity: String,
    pub relation: String,
    pub target: String,
    pub created_at: i64,
}

/// Insert a new structured knowledge edge into the centralized graph.
/// Optionally calculates and stores the semantic vector embedding of the (entity + relation + target) triple.
pub fn insert_knowledge_edge(
    db: &Db,
    entity: &str,
    relation: &str,
    target: &str,
    embedder: Option<&dyn Embedder>,
) -> Result<()> {
    let conn = db.get()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let blob = if let Some(emb) = embedder {
        let triple_text = format!("{} {} {}", entity, relation, target);
        if let Ok(vec) = emb.embed(&triple_text) {
            let mut bytes = Vec::with_capacity(vec.len() * 4);
            for f in &vec {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            Some(bytes)
        } else {
            None
        }
    } else {
        None
    };

    conn.execute(
        "INSERT INTO knowledge_edges (entity, relation, target, embedding, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![entity, relation, target, blob, now],
    )?;

    Ok(())
}

/// List knowledge edges, optionally filtered by entity and/or relation.
pub fn list_knowledge_edges(
    db: &Db,
    entity_filter: Option<&str>,
    relation_filter: Option<&str>,
) -> Result<Vec<KnowledgeEdge>> {
    let conn = db.get()?;
    
    let mut sql = "SELECT id, entity, relation, target, created_at FROM knowledge_edges".to_string();
    let mut conditions = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ent) = entity_filter {
        conditions.push("entity = ?".to_string());
        params_vec.push(Box::new(ent.to_string()));
    }
    if let Some(rel) = relation_filter {
        conditions.push("relation = ?".to_string());
        params_vec.push(Box::new(rel.to_string()));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC");

    let mut stmt = conn.prepare(&sql)?;
    
    // Convert Vec<Box<dyn ToSql>> to a slice of references for rusqlite
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref() as &dyn rusqlite::ToSql).collect();

    let rows = stmt.query_map(&params_refs[..], |row| {
        Ok(KnowledgeEdge {
            id: row.get(0)?,
            entity: row.get(1)?,
            relation: row.get(2)?,
            target: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;

    let mut edges = Vec::new();
    for r in rows {
        edges.push(r?);
    }
    Ok(edges)
}

/// Search the centralized knowledge graph semantically using cosine-similarity over vector embeddings.
pub fn search_knowledge_graph_semantic(
    db: &Db,
    query: &str,
    embedder: &dyn Embedder,
    limit: usize,
) -> Result<Vec<(KnowledgeEdge, f32)>> {
    let query_vec = embedder.embed(query)?;
    if query_vec.is_empty() {
        return Ok(Vec::new());
    }

    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, entity, relation, target, embedding, created_at FROM knowledge_edges
         WHERE embedding IS NOT NULL",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;

    let mut candidates = Vec::new();
    for r in rows {
        let (id, entity, relation, target, blob, created_at) = r?;
        if let Some(emb) = decode_embedding_blob(&blob) {
            if let Some(sim) = cosine_similarity(&query_vec, &emb) {
                candidates.push((
                    KnowledgeEdge {
                        id,
                        entity,
                        relation,
                        target,
                        created_at,
                    },
                    sim,
                ));
            }
        }
    }

    // Sort descending by cosine similarity score
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(limit);

    Ok(candidates)
}

/// Helper function to calculate cosine similarity between two equal-length f32 vectors.
fn cosine_similarity(v1: &[f32], v2: &[f32]) -> Option<f32> {
    if v1.len() != v2.len() || v1.is_empty() {
        return None;
    }
    let mut dot = 0.0;
    let mut norm1 = 0.0;
    let mut norm2 = 0.0;
    for i in 0..v1.len() {
        dot += v1[i] * v2[i];
        norm1 += v1[i] * v1[i];
        norm2 += v2[i] * v2[i];
    }
    if norm1 <= 0.0 || norm2 <= 0.0 {
        return None;
    }
    Some(dot / (norm1.sqrt() * norm2.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open;

    #[test]
    fn test_knowledge_graph_roundtrip() -> Result<()> {
        let db = open(":memory:")?;
        
        insert_knowledge_edge(&db, "main.rs", "calls", "setup_panic_hook", None)?;
        insert_knowledge_edge(&db, "mod.rs", "declares", "knowledge", None)?;
        
        let edges = list_knowledge_edges(&db, None, None)?;
        assert_eq!(edges.len(), 2);
        
        assert_eq!(edges[0].entity, "mod.rs");
        assert_eq!(edges[0].relation, "declares");
        assert_eq!(edges[0].target, "knowledge");

        let filtered = list_knowledge_edges(&db, Some("main.rs"), None)?;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].target, "setup_panic_hook");

        Ok(())
    }
}

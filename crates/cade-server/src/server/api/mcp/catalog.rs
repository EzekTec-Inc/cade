//! Server-owned MCP catalog projection. SQLite is a mirror, never evidence that
//! an external capability is executable. Context, listing and children read the
//! manager's ready catalog; startup, reload and recovery share one mirror writer.
use crate::server::state::McpManager;
use cade_core::capabilities::mesh::{CapabilityExecutionContext, CapabilityMesh};
use cade_store::sqlite::{self, Db, ToolRow};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(crate) fn is_mcp_tool(tool: &ToolRow) -> bool {
    tool.tags
        .iter()
        .any(|tag| tag == "mcp" || tag == "core_mcp")
        || tool.id.starts_with("tool-mcp-")
        || tool
            .json_schema
            .as_ref()
            .is_some_and(|schema| schema["x-cade"]["kind"] == "mcp")
}

fn catalog_rows(
    catalog: Vec<cade_core::capabilities::mesh::TaggedCapabilitySchema>,
) -> Vec<ToolRow> {
    catalog
        .into_iter()
        .filter_map(|tool| {
            let name = tool.schema["name"].as_str()?.to_owned();
            if name.is_empty() {
                return None;
            }
            let description = tool.schema["description"].as_str().map(str::to_owned);
            let stub = cade_agent::agent::tools::build_python_stub_from_schema(
                &name,
                description.as_deref().unwrap_or(""),
                &tool.schema["parameters"],
            );
            Some(ToolRow {
                id: format!("tool-mcp-{name}"),
                name,
                description,
                source_code: Some(stub),
                json_schema: Some(tool.schema),
                tags: tool.tags,
            })
        })
        .collect()
}

/// Persisted native/declarative tools plus the live MCP catalog. Preserve stable
/// IDs for agent attachments, but never let a persisted schema override a peer.
pub(crate) async fn execution_catalog(
    db: &Db,
    mcp: &McpManager,
) -> cade_store::error::Result<Vec<ToolRow>> {
    let mut rows = sqlite::list_tools(db)?;
    let ids: HashMap<_, _> = rows
        .iter()
        .map(|row| (row.name.clone(), row.id.clone()))
        .collect();
    rows.retain(|row| !is_mcp_tool(row));
    let catalog = mcp
        .active_catalog(&CapabilityExecutionContext::new("server-catalog"))
        .await;
    for mut live in catalog_rows(catalog) {
        if let Some(id) = ids.get(&live.name) {
            live.id = id.clone();
        }
        rows.retain(|row| row.name != live.name);
        rows.push(live);
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(rows)
}

/// Reconcile only MCP-owned rows in one transaction. Existing IDs and native
/// tool attachments survive updates. If lifecycle changed during publication,
/// repeat from its latest revision rather than leaving an older snapshot last.
pub async fn sync_mcp_catalog(db: &Db, mcp: &McpManager) -> cade_store::error::Result<()> {
    let mut changes = mcp.subscribe_catalog_changes();
    loop {
        let revision = *changes.borrow_and_update();
        let live = catalog_rows(mcp.settled_catalog().await);
        let names: HashSet<_> = live.iter().map(|row| row.name.as_str()).collect();
        {
            let mut conn = db.get()?;
            let tx = conn.transaction()?;
            let existing = {
                let mut query = tx.prepare("SELECT id, name, tags, json_schema FROM tools")?;
                query
                    .query_map([], |row| {
                        Ok(ToolRow {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            tags: serde_json::from_str(&row.get::<_, String>(2)?)
                                .unwrap_or_default(),
                            json_schema: row
                                .get::<_, Option<String>>(3)?
                                .and_then(|value| serde_json::from_str(&value).ok()),
                            description: None,
                            source_code: None,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for row in existing
                .iter()
                .filter(|row| is_mcp_tool(row) && !names.contains(row.name.as_str()))
            {
                tx.execute("DELETE FROM tools WHERE id = ?1", [&row.id])?;
            }
            for row in &live {
                tx.execute(
                    "INSERT INTO tools (id, name, description, source_code, json_schema, tags, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(name) DO UPDATE SET description=excluded.description,
                         source_code=excluded.source_code, json_schema=excluded.json_schema, tags=excluded.tags",
                    rusqlite::params![row.id, row.name, row.description, row.source_code,
                        row.json_schema.as_ref().map(ToString::to_string),
                        serde_json::to_string(&row.tags).unwrap_or_default(), chrono::Utc::now().timestamp()],
                )?;
            }
            tx.commit()?;
        }
        if *changes.borrow() == revision {
            return Ok(());
        }
    }
}

/// Each server owner starts one subscriber. It also projects the initial state,
/// including the empty catalog, to remove rows left by a previous daemon.
pub fn spawn_catalog_sync(db: Db, mcp: Arc<McpManager>) -> tokio::task::JoinHandle<()> {
    let mut changes = mcp.subscribe_catalog_changes();
    tokio::spawn(async move {
        loop {
            changes.borrow_and_update();
            if let Err(error) = sync_mcp_catalog(&db, &mcp).await {
                tracing::warn!("MCP catalog mirror failed: {error}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            if changes.changed().await.is_err() {
                break;
            }
        }
    })
}

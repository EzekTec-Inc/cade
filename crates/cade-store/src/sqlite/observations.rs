//! P1: Observation capture and retrieval.
//!
//! Observations are lightweight summaries of tool calls that happen during
//! an agentic turn.  They are stored in the `observations` table (migration 7)
//! and injected into the context builder so the LLM has a compressed trail of
//! what it did in past turns — even after those turns have been dropped from
//! the message window.

use super::*;

// ── Row type ─────────────────────────────────────────────────────────────────

/// A single observation record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObservationRow {
    pub id: String,
    pub agent_id: String,
    pub turn: i64,
    pub tool_name: String,
    pub observation_type: String,
    pub summary: String,
    /// JSON array of file paths touched by this tool call.
    pub files: String,
    /// JSON array of concept tags extracted from the call.
    pub concepts: String,
    /// 1–5 importance score (5 = critical, 1 = routine).
    pub importance: i64,
    pub created_at: i64,
}

// ── Insert ───────────────────────────────────────────────────────────────────

/// Record an observation for a tool call.
///
/// `files` and `concepts` should be JSON arrays (e.g. `["src/main.rs"]`).
/// If empty, pass `"[]"`.
#[allow(clippy::too_many_arguments)]
pub fn insert_observation(
    db: &Db,
    agent_id: &str,
    turn: i64,
    tool_name: &str,
    observation_type: &str,
    summary: &str,
    files: &str,
    concepts: &str,
    importance: i64,
) -> Result<String> {
    let id = format!("obs-{}", uuid::Uuid::new_v4());
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO observations (id, agent_id, turn, tool_name, observation_type, summary, files, concepts, importance, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, agent_id, turn, tool_name, observation_type, summary, files, concepts, importance, now_ts()],
    )?;
    Ok(id)
}

// ── Query ────────────────────────────────────────────────────────────────────

/// Fetch the most recent observations for an agent, ordered newest-first.
///
/// `limit` caps the result count (default: 50).
pub fn get_recent_observations(
    db: &Db,
    agent_id: &str,
    limit: usize,
) -> Result<Vec<ObservationRow>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, turn, tool_name, observation_type, summary, files, concepts, importance, created_at
         FROM observations
         WHERE agent_id = ?1
         ORDER BY turn DESC, created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![agent_id, limit as i64], |r| {
        Ok(ObservationRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            turn: r.get(2)?,
            tool_name: r.get(3)?,
            observation_type: r.get(4)?,
            summary: r.get(5)?,
            files: r.get(6)?,
            concepts: r.get(7)?,
            importance: r.get(8)?,
            created_at: r.get(9)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Fetch high-importance observations (importance ≥ `min_importance`) for
/// injection into the context window.  Returns at most `limit` rows,
/// newest-first.
pub fn get_important_observations(
    db: &Db,
    agent_id: &str,
    min_importance: i64,
    limit: usize,
) -> Result<Vec<ObservationRow>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, turn, tool_name, observation_type, summary, files, concepts, importance, created_at
         FROM observations
         WHERE agent_id = ?1 AND importance >= ?2
         ORDER BY turn DESC, created_at DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![agent_id, min_importance, limit as i64], |r| {
        Ok(ObservationRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            turn: r.get(2)?,
            tool_name: r.get(3)?,
            observation_type: r.get(4)?,
            summary: r.get(5)?,
            files: r.get(6)?,
            concepts: r.get(7)?,
            importance: r.get(8)?,
            created_at: r.get(9)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Delete observations older than `max_turn` to prevent unbounded growth.
pub fn prune_old_observations(db: &Db, agent_id: &str, max_turn: i64) -> Result<usize> {
    let conn = db.get()?;
    let deleted = conn.execute(
        "DELETE FROM observations WHERE agent_id = ?1 AND turn < ?2",
        params![agent_id, max_turn],
    )?;
    Ok(deleted)
}

/// Render a compact summary of observations for context injection.
///
/// A6: includes `turn_age` so the agent knows how old each observation is.
///
/// Returns a section like:
/// ```text
/// # Recent Observations (turns 42-55)
/// [turn 55, 0 ago] write_file: Wrote src/main.rs (importance: 4)
/// [turn 42, 13 ago] read_file: Read Cargo.toml (importance: 2)
/// ```
pub fn render_observations_section(observations: &[ObservationRow], budget_chars: usize) -> String {
    if observations.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    let mut total_chars = 0;

    // A6: compute relative time from created_at (epoch seconds).
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for obs in observations {
        let age_secs = (now_epoch - obs.created_at).max(0);
        let age_str = if age_secs < 60 {
            format!("{}s ago", age_secs)
        } else if age_secs < 3600 {
            format!("{}m ago", age_secs / 60)
        } else {
            format!("{}h ago", age_secs / 3600)
        };
        let line = format!(
            "[turn {}, {}] {}: {} (importance: {})",
            obs.turn, age_str, obs.tool_name, obs.summary, obs.importance
        );
        let line_chars = line.chars().count() + 1; // +1 for newline
        if total_chars + line_chars > budget_chars {
            break;
        }
        total_chars += line_chars;
        lines.push(line);
    }

    if lines.is_empty() {
        return String::new();
    }

    // A6: add turn range to header.
    let newest = observations.first().map(|o| o.turn).unwrap_or(0);
    let oldest_shown = observations
        .get(lines.len().saturating_sub(1))
        .map(|o| o.turn)
        .unwrap_or(newest);
    format!(
        "# Recent Observations (turns {}-{})\n{}",
        oldest_shown,
        newest,
        lines.join("\n")
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Result<Db> {
        super::open(":memory:")
    }

    #[test]
    fn insert_and_retrieve_observation() -> Result<()> {
        let db = setup_db()?;
        let agent_id = "agent-obs";
        create_agent(
            &db,
            &AgentRow {
                id: agent_id.to_string(),
                name: "A".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
            },
        )?;

        let id = insert_observation(
            &db,
            agent_id,
            1,
            "read_file",
            "tool_call",
            "Read src/main.rs (120 lines)",
            r#"["src/main.rs"]"#,
            r#"["file_read"]"#,
            3,
        )?;
        assert!(id.starts_with("obs-"));

        let obs = get_recent_observations(&db, agent_id, 10)?;
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].tool_name, "read_file");
        assert_eq!(obs[0].summary, "Read src/main.rs (120 lines)");
        assert_eq!(obs[0].importance, 3);
        Ok(())
    }

    #[test]
    fn get_important_filters_by_importance() -> Result<()> {
        let db = setup_db()?;
        let agent_id = "agent-imp";
        create_agent(
            &db,
            &AgentRow {
                id: agent_id.to_string(),
                name: "A".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
            },
        )?;

        insert_observation(
            &db,
            agent_id,
            1,
            "read_file",
            "tool_call",
            "Read foo",
            "[]",
            "[]",
            2,
        )?;
        insert_observation(
            &db,
            agent_id,
            2,
            "edit_file",
            "tool_call",
            "Edited bar",
            "[]",
            "[]",
            4,
        )?;
        insert_observation(
            &db,
            agent_id,
            3,
            "bash",
            "tool_call",
            "Ran tests",
            "[]",
            "[]",
            5,
        )?;

        let important = get_important_observations(&db, agent_id, 4, 10)?;
        assert_eq!(important.len(), 2);
        assert_eq!(important[0].tool_name, "bash"); // newest first
        assert_eq!(important[1].tool_name, "edit_file");
        Ok(())
    }

    #[test]
    fn prune_removes_old_turns() -> Result<()> {
        let db = setup_db()?;
        let agent_id = "agent-prune";
        create_agent(
            &db,
            &AgentRow {
                id: agent_id.to_string(),
                name: "A".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
            },
        )?;

        insert_observation(&db, agent_id, 1, "a", "tool_call", "old", "[]", "[]", 3)?;
        insert_observation(&db, agent_id, 5, "b", "tool_call", "mid", "[]", "[]", 3)?;
        insert_observation(&db, agent_id, 10, "c", "tool_call", "new", "[]", "[]", 3)?;

        let deleted = prune_old_observations(&db, agent_id, 5)?;
        assert_eq!(deleted, 1); // turn 1 pruned

        let remaining = get_recent_observations(&db, agent_id, 10)?;
        assert_eq!(remaining.len(), 2);
        Ok(())
    }

    #[test]
    fn render_observations_respects_budget() {
        let obs = vec![
            ObservationRow {
                id: "o1".into(),
                agent_id: "a".into(),
                turn: 10,
                tool_name: "read_file".into(),
                observation_type: "tool_call".into(),
                summary: "Read src/main.rs".into(),
                files: "[]".into(),
                concepts: "[]".into(),
                importance: 3,
                created_at: 0,
            },
            ObservationRow {
                id: "o2".into(),
                agent_id: "a".into(),
                turn: 9,
                tool_name: "edit_file".into(),
                observation_type: "tool_call".into(),
                summary: "Edited src/lib.rs".into(),
                files: "[]".into(),
                concepts: "[]".into(),
                importance: 4,
                created_at: 0,
            },
        ];

        // Enough budget for both
        let section = render_observations_section(&obs, 500);
        assert!(section.contains("read_file"));
        assert!(section.contains("edit_file"));
        // A6: header includes turn range
        assert!(section.contains("turns 9-10"));
        // A6: each line includes time-ago
        assert!(section.contains("ago]"));

        // Tiny budget → only first (increase budget to accommodate longer format)
        let section = render_observations_section(&obs, 100);
        assert!(section.contains("read_file"));
        assert!(!section.contains("edit_file"));

        // Empty → empty
        let section = render_observations_section(&[], 500);
        assert!(section.is_empty());
    }
}

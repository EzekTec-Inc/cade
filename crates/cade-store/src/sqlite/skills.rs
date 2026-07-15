//! Skill blacklist — per-agent skill disable/enable.
//!
//! `agent_skill_blacklist` is a simple join table: when a row exists for
//! `(agent_id, skill_id)` the skill is suppressed from that agent's context
//! even if it is installed and discovered.

use crate::error::Result;
use crate::sqlite::Db;

// region:    --- Public API

/// Disable a skill for an agent (add to blacklist).
/// Idempotent — safe to call if the row already exists.
pub fn disable_skill(db: &Db, agent_id: &str, skill_id: &str) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "INSERT OR IGNORE INTO agent_skill_blacklist (agent_id, skill_id) VALUES (?1, ?2)",
        rusqlite::params![agent_id, skill_id],
    )?;
    Ok(())
}

/// Enable a skill for an agent (remove from blacklist).
/// Idempotent — safe to call even if the row does not exist.
pub fn enable_skill(db: &Db, agent_id: &str, skill_id: &str) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "DELETE FROM agent_skill_blacklist WHERE agent_id = ?1 AND skill_id = ?2",
        rusqlite::params![agent_id, skill_id],
    )?;
    Ok(())
}

/// Return the set of skill IDs that are disabled for `agent_id`.
pub fn get_disabled_skills(db: &Db, agent_id: &str) -> Result<Vec<String>> {
    let conn = db.get()?;
    let mut stmt =
        conn.prepare("SELECT skill_id FROM agent_skill_blacklist WHERE agent_id = ?1")?;
    let rows = stmt
        .query_map(rusqlite::params![agent_id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Return `true` if `skill_id` is disabled for `agent_id`.
pub fn is_skill_disabled(db: &Db, agent_id: &str, skill_id: &str) -> Result<bool> {
    let conn = db.get()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM agent_skill_blacklist WHERE agent_id = ?1 AND skill_id = ?2",
        rusqlite::params![agent_id, skill_id],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

// endregion: --- Public API

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{AgentRow, create_agent, open};

    fn mem_db() -> Db {
        open(":memory:").expect("in-memory db")
    }

    fn agent(id: &str) -> AgentRow {
        AgentRow {
            id: id.to_string(),
            name: id.to_string(),
            model: "test".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        }
    }

    #[test]
    fn blacklist_table_exists_after_open() {
        let db = mem_db();
        let conn = db.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_skill_blacklist'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "agent_skill_blacklist table must exist");
    }

    #[test]
    fn disable_adds_row_enable_removes_it() {
        let db = mem_db();
        create_agent(&db, &agent("a1")).unwrap();

        disable_skill(&db, "a1", "rust").unwrap();
        assert!(is_skill_disabled(&db, "a1", "rust").unwrap());

        enable_skill(&db, "a1", "rust").unwrap();
        assert!(!is_skill_disabled(&db, "a1", "rust").unwrap());
    }

    #[test]
    fn disable_is_idempotent() {
        let db = mem_db();
        create_agent(&db, &agent("a2")).unwrap();

        disable_skill(&db, "a2", "tdd-guide").unwrap();
        disable_skill(&db, "a2", "tdd-guide").unwrap(); // second call must not error
        let disabled = get_disabled_skills(&db, "a2").unwrap();
        assert_eq!(disabled.len(), 1);
    }

    #[test]
    fn enable_on_non_disabled_skill_is_safe() {
        let db = mem_db();
        create_agent(&db, &agent("a3")).unwrap();
        // Not disabled, enable should be a no-op
        enable_skill(&db, "a3", "never-disabled").unwrap();
        assert!(!is_skill_disabled(&db, "a3", "never-disabled").unwrap());
    }

    #[test]
    fn get_disabled_skills_returns_all_for_agent() {
        let db = mem_db();
        create_agent(&db, &agent("a4")).unwrap();

        disable_skill(&db, "a4", "skill-a").unwrap();
        disable_skill(&db, "a4", "skill-b").unwrap();

        let mut disabled = get_disabled_skills(&db, "a4").unwrap();
        disabled.sort();
        assert_eq!(disabled, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn blacklist_is_per_agent_not_global() {
        let db = mem_db();
        create_agent(&db, &agent("ag1")).unwrap();
        create_agent(&db, &agent("ag2")).unwrap();

        disable_skill(&db, "ag1", "shared-skill").unwrap();
        // ag2 must not see ag1's blacklist
        assert!(!is_skill_disabled(&db, "ag2", "shared-skill").unwrap());
    }
}

// endregion: --- Tests

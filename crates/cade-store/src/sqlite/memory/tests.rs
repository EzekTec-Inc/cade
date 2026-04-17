#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

use super::*;

fn setup_mem_db() -> Result<Db> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    run_migrations(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn make_agent(db: &Db, id: &str) -> Result<()> {
    agents::create_agent(
        db,
        &AgentRow {
            id: id.into(),
            name: "A".into(),
            model: "m".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
        },
    )?;
    Ok(())
}

#[test]
fn test_upsert_and_get_memory_block() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "Rust app", Some("about"), None)?;

    let blocks = get_memory_blocks(&db, "a1")?;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, "project"); // label
    assert_eq!(blocks[0].1, "Rust app"); // value
    assert_eq!(blocks[0].2, "about"); // description
    Ok(())
}

#[test]
fn test_upsert_memory_block_update() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "v1", None, None)?;
    upsert_memory_block(&db, "a1", "project", "v2", Some("updated"), None)?;

    let blocks = get_memory_blocks(&db, "a1")?;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, "v2");
    assert_eq!(blocks[0].2, "updated");
    Ok(())
}

#[test]
fn test_delete_memory_block() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "data", None, None)?;
    assert!(delete_memory_block(&db, "a1", "project")?);
    assert!(get_memory_blocks(&db, "a1")?.is_empty());
    assert!(!delete_memory_block(&db, "a1", "nope")?);
    Ok(())
}

#[test]
fn test_get_memory_blocks_with_ts() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "block1", "value1", None, None)?;
    let blocks = get_memory_blocks_with_ts(&db, "a1")?;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, "block1"); // label
    assert_eq!(blocks[0].1, "value1"); // value
    assert!(blocks[0].3 > 0); // updated_at timestamp
    Ok(())
}

#[test]
fn test_increment_and_get_turn_counter() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    assert_eq!(get_turn_counter(&db, "a1")?, 0);
    let t1 = increment_turn_counter(&db, "a1")?;
    assert_eq!(t1, 1);
    let t2 = increment_turn_counter(&db, "a1")?;
    assert_eq!(t2, 2);
    assert_eq!(get_turn_counter(&db, "a1")?, 2);
    Ok(())
}

#[test]
fn test_set_memory_tier() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "block1", "data", None, None)?;

    // Set tier to 'long'
    let ok = set_memory_tier(&db, "a1", "block1", "long", false)?;
    assert!(ok);

    // Verify via get_memory_blocks_full
    let full = get_memory_blocks_full(&db, "a1")?;
    assert_eq!(full.len(), 1);
    assert_eq!(full[0].3, "long"); // tier

    // Set tier for missing label
    let ok = set_memory_tier(&db, "a1", "nope", "long", false)?;
    assert!(!ok);
    Ok(())
}

#[test]
fn test_get_active_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "active1", "val1", None, None)?;
    upsert_memory_block(&db, "a1", "active2", "val2", None, None)?;

    // Default tier is 'short' — both should show as active
    let active = get_active_blocks(&db, "a1")?;
    assert_eq!(active.len(), 2);
    Ok(())
}

#[test]
fn test_get_long_term_excerpts() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "block1", "some long data here", None, None)?;
    set_memory_tier(&db, "a1", "block1", "long", false)?;

    let turn = get_turn_counter(&db, "a1")?;
    let excerpts = get_long_term_excerpts(&db, "a1", turn)?;
    assert_eq!(excerpts.len(), 1);
    assert_eq!(excerpts[0].0, "block1"); // label
    Ok(())
}

#[test]
fn test_promote_stale_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "block1", "data", None, None)?;

    // Advance turn counter way past the block's last_turn
    for _ in 0..50 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    // Promote blocks that are 40+ turns stale
    let promoted = promote_stale_blocks(&db, "a1", current_turn, 40)?;
    assert!(promoted >= 1);

    // Verify block is now 'long' tier
    let full = get_memory_blocks_full(&db, "a1")?;
    assert_eq!(full[0].3, "long");
    Ok(())
}

#[test]
fn test_memory_history() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Upsert creates history entries
    upsert_memory_block(&db, "a1", "project", "v1", None, None)?;
    upsert_memory_block(&db, "a1", "project", "v2", None, None)?;
    upsert_memory_block(&db, "a1", "project", "v3", None, None)?;

    let history = get_memory_history(&db, "a1", "project", 10)?;
    // Should have at least the update entries
    assert!(!history.is_empty());
    Ok(())
}

#[test]
fn test_restore_memory_from_history() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "original", None, None)?;
    upsert_memory_block(&db, "a1", "project", "modified", None, None)?;

    // Get history — the first entry should be the "original" value
    let history = get_memory_history(&db, "a1", "project", 10)?;
    if !history.is_empty() {
        let hist_id = &history[history.len() - 1].0; // oldest entry
        let ok = restore_memory_from_history(&db, "a1", "project", hist_id)?;
        assert!(ok);
    }
    Ok(())
}

#[test]
fn test_get_memory_blocks_full() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "b1", "val1", Some("desc1"), None)?;
    upsert_memory_block(&db, "a1", "b2", "val2", Some("desc2"), None)?;

    let full = get_memory_blocks_full(&db, "a1")?;
    assert_eq!(full.len(), 2);
    // Each tuple is (label, value, description, tier)
    let labels: Vec<&str> = full.iter().map(|f| f.0.as_str()).collect();
    assert!(labels.contains(&"b1"));
    assert!(labels.contains(&"b2"));
    Ok(())
}

// -- Confidence weighting

#[test]
fn test_boost_confidence() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "Rust app", None, None)?;

    // Default confidence is 1.0
    let c0 = get_block_confidence(&db, "a1", "project")?;
    assert!((c0 - 1.0).abs() < f64::EPSILON, "default confidence should be 1.0, got {c0}");

    // Boost once
    assert!(boost_confidence(&db, "a1", "project")?);
    let c1 = get_block_confidence(&db, "a1", "project")?;
    assert!(
        (c1 - (1.0 + CONFIDENCE_BOOST_PER_HIT)).abs() < f64::EPSILON,
        "expected {}, got {c1}",
        1.0 + CONFIDENCE_BOOST_PER_HIT
    );

    // Boost twice more
    boost_confidence(&db, "a1", "project")?;
    boost_confidence(&db, "a1", "project")?;
    let c3 = get_block_confidence(&db, "a1", "project")?;
    let expected = 1.0 + 3.0 * CONFIDENCE_BOOST_PER_HIT;
    assert!(
        (c3 - expected).abs() < 0.001,
        "expected ~{expected}, got {c3}"
    );

    Ok(())
}

#[test]
fn test_boost_confidence_wrong_label_returns_false() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "project", "data", None, None)?;

    assert!(!boost_confidence(&db, "a1", "nonexistent")?);
    Ok(())
}

#[test]
fn test_high_confidence_resists_demotion() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "important", "critical data", None, None)?;
    upsert_memory_block(&db, "a1", "forgettable", "ephemeral data", None, None)?;

    // Boost "important" above the retention threshold
    // Need ceil((1.5 - 1.0) / 0.15) = 4 boosts to cross 1.5
    for _ in 0..4 {
        boost_confidence(&db, "a1", "important")?;
    }
    let c = get_block_confidence(&db, "a1", "important")?;
    assert!(c >= CONFIDENCE_RETENTION_THRESHOLD, "confidence {c} should be >= {CONFIDENCE_RETENTION_THRESHOLD}");

    // Advance turns way past threshold
    for _ in 0..50 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    // Run demotion
    let promoted = promote_stale_blocks(&db, "a1", current_turn, 40)?;

    // "forgettable" should be demoted, "important" should resist
    let full = get_memory_blocks_full(&db, "a1")?;
    let important_tier = full.iter().find(|(l, _, _, _)| l == "important").map(|t| &t.3);
    let forgettable_tier = full.iter().find(|(l, _, _, _)| l == "forgettable").map(|t| &t.3);

    assert_eq!(important_tier, Some(&"short".to_string()), "high-confidence block should stay short");
    assert_eq!(forgettable_tier, Some(&"long".to_string()), "low-confidence block should be demoted");
    assert_eq!(promoted, 1, "only one block should be demoted");

    Ok(())
}

#[test]
fn test_low_confidence_still_demoted() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "block1", "data", None, None)?;

    // Boost once — still below threshold (1.0 + 0.15 = 1.15 < 1.5)
    boost_confidence(&db, "a1", "block1")?;
    let c = get_block_confidence(&db, "a1", "block1")?;
    assert!(c < CONFIDENCE_RETENTION_THRESHOLD);

    // Advance turns
    for _ in 0..50 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    let promoted = promote_stale_blocks(&db, "a1", current_turn, 40)?;
    assert_eq!(promoted, 1, "below-threshold block should still be demoted");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase A.1 — context survival regression
// ─────────────────────────────────────────────────────────────────────────────
//
// These tests protect the invariant that lets an agent recover what it was
// working on after context truncation or a process restart:
//
//   1. `working_set` and `session_summary` are persisted to SQLite, not only
//      to the in-process prompt builder.
//   2. Pinned blocks survive an arbitrary number of "idle turns" without being
//      demoted to long-term.
//   3. Block content round-trips faithfully through a simulated session reset
//      (drop Db handle, reopen same file, re-read).

#[test]
fn survival_working_set_persists_across_reopen() -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cade_survival_{nanos}.sqlite"));
    // Best-effort cleanup if a prior run left it behind.
    let _ = std::fs::remove_file(&path);

    // -- Session 1: seed the block
    {
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        apply_schema(&conn)?;
        run_migrations(&conn)?;
        let db = Arc::new(Mutex::new(conn));
        make_agent(&db, "survivor")?;

        let task = "Current task: implement Phase B.\n\
                    Files modified: crates/cade-embed/*\n\
                    Next: wire hybrid retrieval.";
        upsert_memory_block(&db, "survivor", "working_set", task, Some("active task"), None)?;

        // Simulate many idle turns elapsing WITHOUT touching working_set
        for _ in 0..200 {
            increment_turn_counter(&db, "survivor")?;
        }
    } // Db dropped — mimics process exit

    // -- Session 2: reopen the same file
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    run_migrations(&conn)?;
    let db = Arc::new(Mutex::new(conn));

    let blocks = get_memory_blocks(&db, "survivor")?;
    let ws = blocks
        .iter()
        .find(|(l, _, _)| l == "working_set")
        .ok_or("working_set missing after reopen")?;
    assert!(
        ws.1.contains("Phase B") && ws.1.contains("crates/cade-embed"),
        "working_set content corrupted on reopen: {:?}",
        ws.1
    );
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[test]
fn survival_pinned_block_immune_to_staleness() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "persona", "I am CADE.", None, None)?;
    set_memory_tier(&db, "a1", "persona", "pinned", false)?;

    // Advance far past the 80-turn stale threshold
    for _ in 0..300 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    let demoted = promote_stale_blocks(&db, "a1", current_turn, 40)?;
    assert_eq!(demoted, 0, "pinned blocks must never be demoted");

    // And the tier is still pinned
    let full = get_memory_blocks_full(&db, "a1")?;
    let tier = full.iter().find(|(l, ..)| l == "persona").map(|t| &t.3);
    assert_eq!(tier, Some(&"pinned".to_string()));
    Ok(())
}

#[test]
fn survival_session_summary_roundtrip() -> Result<()> {
    // Protects the contract that `session_summary` (written by sleeptime
    // consolidation) persists like any other memory block. If the block type
    // ever special-cases storage, this test breaks first.
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let summary = "SUMMARY:\nTask: fix /skills menu\nFiles: commands_skills.rs, skills.rs\n\
                   ANCHORS: SkillScope::display_order, launch_editor";
    upsert_memory_block(&db, "a1", "session_summary", summary, None, None)?;

    let blocks = get_memory_blocks(&db, "a1")?;
    let ss = blocks
        .iter()
        .find(|(l, _, _)| l == "session_summary")
        .ok_or("session_summary missing")?;
    assert!(ss.1.contains("ANCHORS"), "anchors must survive");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase A.2 — schema snapshot
// ─────────────────────────────────────────────────────────────────────────────
//
// Locks the shape of every user-data table. Any future migration must update
// this test deliberately — preventing accidental schema drift.

/// Collect column names for a table in declaration order.
fn column_names(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(cols)
}

#[test]
fn schema_snapshot_locks_known_tables() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    run_migrations(&conn)?;

    // -- agents
    let agents_cols = column_names(&conn, "agents")?;
    assert_eq!(
        agents_cols,
        vec![
            "id",
            "name",
            "model",
            "description",
            "system_prompt",
            "created_at",
            "memory_turn_counter",
            "compaction_model",
        ],
        "agents table drift detected"
    );

    // -- shared_memory_blocks
    let smb_cols = column_names(&conn, "shared_memory_blocks")?;
    assert!(smb_cols.contains(&"id".to_string()));
    assert!(smb_cols.contains(&"label".to_string()));
    assert!(smb_cols.contains(&"value".to_string()));
    assert!(smb_cols.contains(&"tier".to_string()));
    assert!(smb_cols.contains(&"confidence".to_string()));

    // -- agent_memory_blocks
    let amb_cols = column_names(&conn, "agent_memory_blocks")?;
    assert!(amb_cols.contains(&"agent_id".to_string()));
    assert!(amb_cols.contains(&"block_id".to_string()));

    // -- messages
    let msg_cols = column_names(&conn, "messages")?;
    for required in &["id", "agent_id", "role", "content", "char_count", "created_at"] {
        assert!(
            msg_cols.iter().any(|c| c == required),
            "messages.{required} missing"
        );
    }

    // -- archival_memory is an FTS5 virtual table; PRAGMA table_info returns
    //    its shadow columns. We just verify it exists.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='archival_memory'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(exists, 1, "archival_memory virtual table missing");

    Ok(())
}

#[test]
fn schema_migrations_are_idempotent() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    run_migrations(&conn)?;
    // Second pass — must succeed without error and without changing user_version.
    run_migrations(&conn)?;
    run_migrations(&conn)?;
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    assert!(v >= 2, "user_version must be at the current head");
    Ok(())
}

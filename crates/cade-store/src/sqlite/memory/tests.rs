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
            compaction_model: None, theme: None,
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

/// Bug 5: get_active_blocks must exclude long-tier blocks so that subagent
/// memory seeding only inherits pinned + short (active context), not stale
/// archived blocks that would waste the subagent's context window.
#[test]
fn get_active_blocks_excludes_long_tier() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "rust project", None, None)?;
    upsert_memory_block(&db, "a1", "archived_stuff", "old data", None, None)?;

    // Demote one block to long tier
    set_memory_tier(&db, "a1", "archived_stuff", "long", false)?;

    let active = get_active_blocks(&db, "a1")?;
    assert_eq!(active.len(), 1, "only pinned+short blocks should be returned");
    assert_eq!(active[0].0, "project");
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
    assert_eq!(excerpts[0].label, "block1");
    assert!(excerpts[0].char_count > 0);
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
//   1. `active_goal` and `session_summary` are persisted to SQLite, not only
//      to the in-process prompt builder.
//   2. Pinned blocks survive an arbitrary number of "idle turns" without being
//      demoted to long-term.
//   3. Block content round-trips faithfully through a simulated session reset
//      (drop Db handle, reopen same file, re-read).

#[test]
fn survival_active_goal_persists_across_reopen() -> Result<()> {
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
        upsert_memory_block(&db, "survivor", "active_goal", task, Some("active task"), None)?;

        // Simulate many idle turns elapsing WITHOUT touching active_goal
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
        .find(|(l, _, _)| l == "active_goal")
        .ok_or("active_goal missing after reopen")?;
    assert!(
        ws.1.contains("Phase B") && ws.1.contains("crates/cade-embed"),
        "active_goal content corrupted on reopen: {:?}",
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
            "theme",
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

// ─────────────────────────────────────────────────────────────────────────────
// Phase B — export to rag-indexable directory
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn export_round_trips_blocks_and_archival() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "persona", "I am CADE.", Some("identity"), None)?;
    upsert_memory_block(&db, "a1", "project", "rust workspace", None, None)?;
    set_memory_tier(&db, "a1", "persona", "pinned", false)?;

    crate::sqlite::tools::insert_archival_memory(
        &db,
        "a1",
        "The quick brown fox jumps over the lazy dog.",
        &["english".into(), "pangram".into()],
    )?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let out_dir = std::env::temp_dir().join(format!("cade_export_{nanos}"));

    let report = export_memory_to_rag_dir(&db, "a1", &out_dir)?;
    assert_eq!(report.blocks_written, 2);
    assert_eq!(report.archival_written, 1);

    // -- Verify block content + front-matter
    let persona_path = out_dir.join("blocks").join("persona.md");
    let persona = std::fs::read_to_string(&persona_path)?;
    assert!(persona.contains("label: persona"));
    assert!(persona.contains("tier: pinned"));
    assert!(persona.contains("I am CADE."));

    let project_path = out_dir.join("blocks").join("project.md");
    let project = std::fs::read_to_string(&project_path)?;
    assert!(project.contains("tier: short")); // default tier
    assert!(project.contains("rust workspace"));

    // -- Verify archival
    let archival_dir = out_dir.join("archival");
    let entries: Vec<_> = std::fs::read_dir(&archival_dir)?.collect::<std::result::Result<_, _>>()?;
    assert_eq!(entries.len(), 1);
    let archival_body = std::fs::read_to_string(entries[0].path())?;
    assert!(archival_body.contains("tier: archival"));
    assert!(archival_body.contains("tags: [\"english\",\"pangram\"]"));
    assert!(archival_body.contains("brown fox"));

    // -- Cleanup
    let _ = std::fs::remove_dir_all(&out_dir);
    Ok(())
}

#[test]
fn export_removes_stale_files_on_rerun() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "foo", "v1", None, None)?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let out_dir = std::env::temp_dir().join(format!("cade_export_stale_{nanos}"));

    export_memory_to_rag_dir(&db, "a1", &out_dir)?;
    assert!(out_dir.join("blocks").join("foo.md").exists());

    // Delete `foo`, add `bar`
    delete_memory_block(&db, "a1", "foo")?;
    upsert_memory_block(&db, "a1", "bar", "v1", None, None)?;

    export_memory_to_rag_dir(&db, "a1", &out_dir)?;
    assert!(
        !out_dir.join("blocks").join("foo.md").exists(),
        "stale file for deleted block must not persist"
    );
    assert!(out_dir.join("blocks").join("bar.md").exists());

    let _ = std::fs::remove_dir_all(&out_dir);
    Ok(())
}

#[test]
fn export_sanitizes_pathological_labels() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Labels containing filesystem-unsafe characters.
    upsert_memory_block(&db, "a1", "skill:rust", "content-a", None, None)?;
    upsert_memory_block(&db, "a1", "path/traversal", "content-b", None, None)?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let out_dir = std::env::temp_dir().join(format!("cade_export_safe_{nanos}"));

    let report = export_memory_to_rag_dir(&db, "a1", &out_dir)?;
    assert_eq!(report.blocks_written, 2);

    // Neither pathological path escaped the blocks/ directory
    let blocks_dir = out_dir.join("blocks");
    let entries: Vec<_> = std::fs::read_dir(&blocks_dir)?.collect::<std::result::Result<_, _>>()?;
    assert_eq!(entries.len(), 2);
    for e in &entries {
        let name = e.file_name().into_string().unwrap();
        assert!(!name.contains('/'), "filename must not contain /");
        assert!(!name.contains(':'), "filename must not contain :");
    }

    let _ = std::fs::remove_dir_all(&out_dir);
    Ok(())
}


// ─────────────────────────────────────────────────────────────────────────────
// M1 — active_goal auto-pin on first non-empty write
// ─────────────────────────────────────────────────────────────────────────────
//
// Contract: when an agent first writes a non-empty value to `active_goal`,
// `upsert_memory_block` must flip the block's tier from the default `short`
// to `pinned` so that `promote_stale_blocks` never archives it. This closes
// the race where `active_goal` could be demoted to `long` before
// `consolidate_agent` had a chance to pin it.
//
// Invariants preserved:
//   * Seed (empty-value) writes leave tier = `short` (so `DEFAULT_MEMORY_BLOCKS`
//     seeding does not flip the tier on the initial blank insert).
//   * No other labels are affected by this auto-pin rule.
//   * Once pinned, subsequent writes stay pinned (never downgraded).

#[test]
fn m1_active_goal_auto_pins_on_first_nonempty_write() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "active_goal", "Active task: implement M1", None, None)?;

    let active = get_active_blocks(&db, "a1")?;
    let (_, _, _, tier, _) = active
        .iter()
        .find(|(l, _, _, _, _)| l == "active_goal")
        .expect("active_goal must exist after upsert");
    assert_eq!(
        tier, "pinned",
        "active_goal must be auto-pinned on first non-empty write"
    );
    Ok(())
}

#[test]
fn m1_active_goal_empty_seed_stays_short() -> Result<()> {
    // Seeding step in bootstrap writes empty values from DEFAULT_MEMORY_BLOCKS.
    // That initial blank write MUST NOT flip the tier to pinned — the existing
    // `r06_active_goal_is_short_not_pinned` invariant depends on tier being
    // `short` after seed.
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "active_goal", "", None, None)?;

    let active = get_active_blocks(&db, "a1")?;
    let (_, _, _, tier, _) = active
        .iter()
        .find(|(l, _, _, _, _)| l == "active_goal")
        .expect("active_goal must exist after empty seed");
    assert_eq!(
        tier, "short",
        "empty-value seed must leave active_goal in short tier"
    );
    Ok(())
}

#[test]
fn m1_active_goal_whitespace_only_value_stays_short() -> Result<()> {
    // Whitespace-only values are effectively empty — they must not pin.
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "active_goal", "   \n\t  ", None, None)?;

    let active = get_active_blocks(&db, "a1")?;
    let (_, _, _, tier, _) = active
        .iter()
        .find(|(l, _, _, _, _)| l == "active_goal")
        .expect("active_goal must exist");
    assert_eq!(
        tier, "short",
        "whitespace-only value must leave active_goal in short tier"
    );
    Ok(())
}

#[test]
fn m1_other_labels_are_not_auto_pinned() -> Result<()> {
    // Only `active_goal` is auto-pinned. `project`, `persona`, `human`, custom
    // labels — all must still default to `short` on first write. (The existing
    // bootstrap code explicitly pins `persona`/`human`/`project` via
    // set_memory_tier AFTER insert; we must not short-circuit that via
    // upsert_memory_block.)
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "Rust coding assistant", None, None)?;
    upsert_memory_block(&db, "a1", "session_summary", "recap text", None, None)?;
    upsert_memory_block(&db, "a1", "some_custom_label", "payload", None, None)?;

    let active = get_active_blocks(&db, "a1")?;
    for (label, _, _, tier, _) in &active {
        if label == "active_goal" {
            continue;
        }
        assert_eq!(
            tier, "short",
            "label '{label}' must default to short tier (only active_goal auto-pins)"
        );
    }
    Ok(())
}

#[test]
fn m1_active_goal_remains_pinned_on_subsequent_writes() -> Result<()> {
    // Once pinned, further upserts must keep tier = pinned. The existing
    // CASE WHEN tier = 'pinned' THEN 'pinned' ELSE 'short' END clause in the
    // UPDATE path already protects this, but a regression test guards it.
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "active_goal", "v1", None, None)?;
    upsert_memory_block(&db, "a1", "active_goal", "v2", None, None)?;

    let active = get_active_blocks(&db, "a1")?;
    let (_, value, _, tier, _) = active
        .iter()
        .find(|(l, _, _, _, _)| l == "active_goal")
        .expect("active_goal must exist");
    assert_eq!(tier, "pinned");
    assert_eq!(value, "v2");
    Ok(())
}

// ── Shared / standalone block tests ──────────────────────────────────────────

#[test]
fn test_create_standalone_block() -> Result<()> {
    let db = setup_mem_db()?;
    let id = create_standalone_block(&db, "org_policy", "No tabs", Some("style guide"), None)?;
    assert!(!id.is_empty());

    // Block exists in shared_memory_blocks
    let info = get_block_by_id(&db, &id)?.expect("block must exist");
    assert_eq!(info.label, "org_policy");
    assert_eq!(info.value, "No tabs");
    assert_eq!(info.description, "style guide");
    assert_eq!(info.tier, "short");

    // NOT in any agent junction
    let agents = list_agents_for_block(&db, &id)?;
    assert!(agents.is_empty(), "standalone block must not be attached to any agent");
    Ok(())
}

#[test]
fn test_get_block_by_id_not_found() -> Result<()> {
    let db = setup_mem_db()?;
    assert!(get_block_by_id(&db, "nonexistent")?.is_none());
    Ok(())
}

#[test]
fn test_list_all_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    let id1 = create_standalone_block(&db, "alpha", "v1", None, None)?;
    let id2 = create_standalone_block(&db, "beta", "v2", None, None)?;

    let all = list_all_blocks(&db, None)?;
    assert_eq!(all.len(), 2);
    // Both blocks present (order may vary when timestamps are identical)
    let ids: Vec<&str> = all.iter().map(|b| b.id.as_str()).collect();
    assert!(ids.contains(&id1.as_str()));
    assert!(ids.contains(&id2.as_str()));
    Ok(())
}

#[test]
fn test_list_all_blocks_with_filter() -> Result<()> {
    let db = setup_mem_db()?;
    create_standalone_block(&db, "alpha", "v1", None, None)?;
    let id2 = create_standalone_block(&db, "beta", "v2", None, None)?;

    let filtered = list_all_blocks(&db, Some("beta"))?;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, id2);

    let empty = list_all_blocks(&db, Some("nope"))?;
    assert!(empty.is_empty());
    Ok(())
}

#[test]
fn test_delete_block_permanently() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let id = create_standalone_block(&db, "temp", "data", None, None)?;
    link_shared_memory_block(&db, "a1", &id)?;
    assert_eq!(list_agents_for_block(&db, &id)?.len(), 1);

    // Permanent delete cascades junction rows
    assert!(delete_block_permanently(&db, &id)?);
    assert!(get_block_by_id(&db, &id)?.is_none());
    assert!(list_agents_for_block(&db, &id)?.is_empty());

    // Idempotent — second delete returns false
    assert!(!delete_block_permanently(&db, &id)?);
    Ok(())
}

#[test]
fn test_unlink_shared_memory_block() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let id = create_standalone_block(&db, "shared", "val", None, None)?;
    link_shared_memory_block(&db, "a1", &id)?;

    // Agent sees the block
    let blocks = get_memory_blocks(&db, "a1")?;
    assert_eq!(blocks.len(), 1);

    // Unlink removes agent view but block survives
    assert!(unlink_shared_memory_block(&db, "a1", &id)?);
    let blocks = get_memory_blocks(&db, "a1")?;
    assert!(blocks.is_empty());
    assert!(get_block_by_id(&db, &id)?.is_some(), "block must still exist");

    // Second unlink returns false
    assert!(!unlink_shared_memory_block(&db, "a1", &id)?);
    Ok(())
}

#[test]
fn test_list_agents_for_block() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    make_agent(&db, "a2")?;

    let id = create_standalone_block(&db, "shared_kb", "facts", None, None)?;
    link_shared_memory_block(&db, "a1", &id)?;
    link_shared_memory_block(&db, "a2", &id)?;

    let agents = list_agents_for_block(&db, &id)?;
    assert_eq!(agents, vec!["a1", "a2"]);
    Ok(())
}

#[test]
fn test_get_memory_blocks_with_ids() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "project", "Rust app", Some("desc"), None)?;
    upsert_memory_block(&db, "a1", "persona", "Helpful", None, None)?;

    let rows = get_memory_blocks_with_ids(&db, "a1")?;
    assert_eq!(rows.len(), 2);
    // Ordered by label: persona < project
    assert_eq!(rows[0].1, "persona");
    assert_eq!(rows[1].1, "project");
    // block_id is non-empty UUID
    assert!(!rows[0].0.is_empty());
    assert!(!rows[1].0.is_empty());
    assert_ne!(rows[0].0, rows[1].0, "each block gets a unique ID");
    Ok(())
}

#[test]
fn test_shared_block_cross_agent_live_sync() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    make_agent(&db, "a2")?;

    // Agent 1 creates a block
    upsert_memory_block(&db, "a1", "team_info", "Initial", None, None)?;

    // Find the block_id
    let rows = get_memory_blocks_with_ids(&db, "a1")?;
    let block_id = &rows[0].0;

    // Attach to agent 2
    link_shared_memory_block(&db, "a2", block_id)?;

    // Both see the same value
    let b1 = get_memory_blocks(&db, "a1")?;
    let b2 = get_memory_blocks(&db, "a2")?;
    assert_eq!(b1[0].1, "Initial");
    assert_eq!(b2[0].1, "Initial");

    // Agent 2 updates
    upsert_memory_block(&db, "a2", "team_info", "Updated by A2", None, None)?;

    // Agent 1 sees the update (live sync)
    let b1 = get_memory_blocks(&db, "a1")?;
    assert_eq!(b1[0].1, "Updated by A2");
    Ok(())
}

#[test]
fn test_standalone_block_with_max_chars() -> Result<()> {
    let db = setup_mem_db()?;
    let id = create_standalone_block(&db, "limited", "hi", None, Some(100))?;
    let info = get_block_by_id(&db, &id)?.unwrap();
    assert_eq!(info.max_chars, Some(100));
    Ok(())
}

// ── Typed confidence boost ────────────────────────────────────────────────

/// Typed memory blocks of types `decision`, `constraint`, and `convention`
/// should receive a higher initial confidence so they resist archival
/// demotion with fewer (or zero) search hits.
#[test]
fn typed_decision_block_gets_confidence_boost() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block_typed(
        &db, "a1", "db_choice", "use postgres", None, None,
        Some("decision"), None,
    )?;

    let c = get_block_confidence(&db, "a1", "db_choice")?;
    assert!(
        c > 1.0,
        "decision block should have confidence > 1.0 (default), got {c}"
    );
    Ok(())
}

#[test]
fn typed_constraint_block_gets_confidence_boost() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block_typed(
        &db, "a1", "no_new_deps", "no new dependencies without approval", None, None,
        Some("constraint"), None,
    )?;

    let c = get_block_confidence(&db, "a1", "no_new_deps")?;
    assert!(
        c > 1.0,
        "constraint block should have confidence > 1.0 (default), got {c}"
    );
    Ok(())
}

#[test]
fn typed_convention_block_gets_confidence_boost() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block_typed(
        &db, "a1", "commit_style", "use conventional commits", None, None,
        Some("convention"), None,
    )?;

    let c = get_block_confidence(&db, "a1", "commit_style")?;
    assert!(
        c > 1.0,
        "convention block should have confidence > 1.0 (default), got {c}"
    );
    Ok(())
}

#[test]
fn typed_generic_block_does_not_get_boost() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block_typed(
        &db, "a1", "random_note", "some note", None, None,
        Some("generic"), None,
    )?;

    let c = get_block_confidence(&db, "a1", "random_note")?;
    assert!(
        (c - 1.0).abs() < f64::EPSILON,
        "generic block should keep default confidence 1.0, got {c}"
    );
    Ok(())
}

/// A decision block with a single search_memory hit should reach the
/// retention threshold (1.5), making it immune to archival.
#[test]
fn typed_decision_resists_demotion_after_one_search_hit() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block_typed(
        &db, "a1", "arch_choice", "chose microservices", None, None,
        Some("decision"), None,
    )?;

    // One search hit
    boost_confidence(&db, "a1", "arch_choice")?;
    let c = get_block_confidence(&db, "a1", "arch_choice")?;
    assert!(
        c >= CONFIDENCE_RETENTION_THRESHOLD,
        "decision + 1 boost should reach retention threshold {CONFIDENCE_RETENTION_THRESHOLD}, got {c}"
    );

    // Advance turns way past stale threshold
    for _ in 0..100 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    let promoted = promote_stale_blocks(&db, "a1", current_turn, 80)?;
    assert_eq!(promoted, 0, "decision block with one search hit should resist demotion");

    Ok(())
}

// ── F7: activity-weighted aging tests ───────────────────────────────────────

/// F7: bump_block_access on a block must increment access_count and stamp
/// last_access_turn to the current turn counter.
#[test]
fn f7_bump_block_access_increments_count_and_stamp() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "tracked", "value", None, None)?;

    // Advance turn counter so last_access_turn is meaningfully > 0
    for _ in 0..7 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    bump_block_access(&db, "a1", &["tracked"]);
    bump_block_access(&db, "a1", &["tracked"]);
    bump_block_access(&db, "a1", &["tracked"]);

    // Read back access_count + last_access_turn directly.
    let conn = db.lock();
    let (count, stamp): (i64, i64) = conn.query_row(
        "SELECT access_count, last_access_turn
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        params!["a1", "tracked"],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    assert_eq!(count, 3, "access_count must reflect three reads");
    assert_eq!(stamp, current_turn, "last_access_turn must equal current turn");
    Ok(())
}

/// F7: a block that has been accessed many times survives the standard
/// staleness threshold even though it's never been re-written.
#[test]
fn f7_high_access_block_resists_demotion() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "frequent", "data", None, None)?;

    // Bump access 10 times — caps the boost at 3× the base threshold.
    for _ in 0..10 {
        bump_block_access(&db, "a1", &["frequent"]);
    }

    // Advance turn counter to just past the base threshold (not the boosted one).
    // Base threshold = 80; with 10 accesses the effective threshold is 240.
    // Stamp `last_access_turn` at turn N, then advance 100 turns: idle = 100 < 240.
    bump_block_access(&db, "a1", &["frequent"]); // resets last_access_turn
    let stamp_turn = get_turn_counter(&db, "a1")?;
    for _ in 0..100 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;
    assert_eq!(current_turn - stamp_turn, 100);

    let promoted = promote_stale_blocks(&db, "a1", current_turn, 80)?;
    assert_eq!(
        promoted, 0,
        "F7: block with 10 accesses must survive 100 idle turns when base threshold is 80"
    );

    let full = get_memory_blocks_full(&db, "a1")?;
    let frequent = full
        .iter()
        .find(|(l, _, _, _)| l == "frequent")
        .expect("frequent block must still exist");
    assert_eq!(
        frequent.3, "short",
        "frequent block must remain in short tier"
    );
    Ok(())
}

/// F7: a block that has NEVER been accessed gets demoted on the standard
/// schedule — the access boost is purely additive.
#[test]
fn f7_zero_access_block_demotes_on_base_threshold() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "ignored", "data", None, None)?;

    for _ in 0..100 {
        increment_turn_counter(&db, "a1")?;
    }
    let current_turn = get_turn_counter(&db, "a1")?;

    let promoted = promote_stale_blocks(&db, "a1", current_turn, 80)?;
    assert_eq!(
        promoted, 1,
        "F7: zero-access block must demote on the base threshold"
    );
    Ok(())
}

/// F7: bumping access on one agent's block must not leak access counts to
/// the same shared block on another agent.
#[test]
fn f7_access_bump_is_scoped_to_agent() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    make_agent(&db, "a2")?;
    // Same label on both agents, distinct rows (cross-agent shared block
    // semantics are tested elsewhere — here we just want two separate ids).
    upsert_memory_block(&db, "a1", "shared_label", "for-a1", None, None)?;
    upsert_memory_block(&db, "a2", "shared_label", "for-a2", None, None)?;

    bump_block_access(&db, "a1", &["shared_label"]);
    bump_block_access(&db, "a1", &["shared_label"]);

    let conn = db.lock();
    let a1_count: i64 = conn.query_row(
        "SELECT access_count FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'shared_label'",
        [],
        |r| r.get(0),
    )?;
    let a2_count: i64 = conn.query_row(
        "SELECT access_count FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a2' AND b.label = 'shared_label'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(a1_count, 2, "a1's block must have 2 accesses");
    assert_eq!(a2_count, 0, "a2's block must NOT have inherited any accesses");
    Ok(())
}

// ── A2: Write-ahead verification ──────────────────────────────────────────

#[test]
fn upsert_returns_write_result_with_char_counts() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let wr = upsert_memory_block(&db, "a1", "note", "hello world", None, None)?;
    assert!(!wr.was_truncated);
    assert_eq!(wr.stored_chars, 11);
    assert_eq!(wr.requested_chars, 11);
    Ok(())
}

#[test]
fn upsert_truncates_when_exceeding_char_limit() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // A1: exceeding max_chars now truncates instead of erroring
    let big = "x".repeat(200);
    let wr = upsert_memory_block(&db, "a1", "note", &big, None, Some(100))?;
    assert!(wr.was_truncated);
    assert_eq!(wr.stored_chars, 100);
    assert_eq!(wr.requested_chars, 200);

    // Verify the stored value is actually truncated
    let blocks = get_memory_blocks(&db, "a1")?;
    let stored = blocks.iter().find(|(l, _, _)| l == "note").unwrap();
    assert_eq!(stored.1.chars().count(), 100);
    Ok(())
}

#[test]
fn extract_keywords_returns_distinctive_terms() -> Result<()> {
    use super::extract_keywords;
    
    let text = "The Rust programming language is a systems programming language that focuses on memory safety. Rust provides zero-cost abstractions and memory management without garbage collection.";
    let keywords = extract_keywords(text, 5);
    
    // Should extract distinctive terms, not stop words
    assert_eq!(keywords.len(), 5);
    assert!(keywords.contains(&"programming".to_string()));
    assert!(keywords.contains(&"rust".to_string()) || keywords.contains(&"memory".to_string()));
    assert!(!keywords.contains(&"the".to_string()));
    assert!(!keywords.contains(&"is".to_string()));
    
    // Empty text should return empty vec
    let empty_keywords = extract_keywords("", 5);
    assert!(empty_keywords.is_empty());
    
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// WI-SEMANTIC Phase 1: memory_blocks_fts virtual table (Migration 10)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn memory_blocks_fts_exists_after_migration() -> Result<()> {
    let db = setup_mem_db()?;
    let conn = db.lock();
    // Virtual table is registered in sqlite_master with type='table'
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_blocks_fts'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 1, "memory_blocks_fts virtual table must exist after migrations");
    Ok(())
}

#[test]
fn memory_blocks_fts_indexes_upserted_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(
        &db,
        "a1",
        "deadlock_fix",
        "We scoped the parking_lot mutex lock to a smaller block",
        None,
        None,
    )?;

    let conn = db.lock();
    let hits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_blocks_fts WHERE memory_blocks_fts MATCH ?1",
        rusqlite::params!["mutex"],
        |r| r.get(0),
    )?;
    assert!(hits >= 1, "FTS must index value text — got {hits} hits for 'mutex'");
    Ok(())
}

#[test]
fn search_memory_blocks_fts_returns_memory_hits_not_messages() -> Result<()> {
    use crate::sqlite::embedding::search_memory_blocks_fts;
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(
        &db,
        "a1",
        "rust_tip",
        "Use parking_lot::Mutex for lower contention overhead",
        None,
        None,
    )?;

    let conn = db.lock();
    let hits = search_memory_blocks_fts(&conn, "a1", "parking_lot", 10)?;
    assert!(
        !hits.is_empty(),
        "search_memory_blocks_fts must return memory hits, got 0"
    );
    let labels: Vec<&str> = hits.iter().map(|(_, _, l, _)| l.as_str()).collect();
    assert!(labels.contains(&"rust_tip"), "expected 'rust_tip' in {labels:?}");
    Ok(())
}

#[test]
fn shared_memory_blocks_has_embedding_column() -> Result<()> {
    let db = setup_mem_db()?;
    let conn = db.lock();
    // PRAGMA table_info returns one row per column with name in col 1.
    let mut stmt = conn.prepare("PRAGMA table_info(shared_memory_blocks)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    assert!(
        cols.contains(&"embedding".to_string()),
        "embedding column missing from shared_memory_blocks; got {cols:?}"
    );
    Ok(())
}

#[test]
fn upsert_with_embedder_writes_blob() -> Result<()> {
    use crate::sqlite::embedding::Embedder;
    use crate::sqlite::memory::upsert_memory_block_with_embedder;

    // A minimal deterministic embedder for the test (no fastembed dependency).
    struct FakeEmbedder;
    impl Embedder for FakeEmbedder {
        fn embed(&self, _text: &str) -> crate::error::Result<Vec<f32>> {
            Ok(vec![0.25_f32, 0.5, 0.75, 1.0])
        }
        fn dimension(&self) -> usize {
            4
        }
    }

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block_with_embedder(
        &db,
        "a1",
        "lock_fix",
        "scoped parking_lot mutex to a smaller block",
        None,
        None,
        Some(&FakeEmbedder),
    )?;

    let conn = db.lock();
    let blob: Option<Vec<u8>> = conn.query_row(
        "SELECT b.embedding FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        rusqlite::params!["a1", "lock_fix"],
        |r| r.get(0),
    )?;
    let bytes = blob.expect("embedding BLOB must be set");
    assert_eq!(bytes.len(), 4 * 4, "expected 4 f32 little-endian bytes = 16");

    // Decode and verify the f32 values round-tripped.
    let mut floats = Vec::with_capacity(4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().expect("chunks_exact(4)");
        floats.push(f32::from_le_bytes(arr));
    }
    assert_eq!(floats, vec![0.25_f32, 0.5, 0.75, 1.0]);
    Ok(())
}

#[test]
fn upsert_with_none_embedder_leaves_embedding_null() -> Result<()> {
    use crate::sqlite::embedding::Embedder;
    use crate::sqlite::memory::upsert_memory_block_with_embedder;

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block_with_embedder(
        &db,
        "a1",
        "no_emb",
        "value without embedder",
        None,
        None,
        None::<&dyn Embedder>,
    )?;

    let conn = db.lock();
    let blob: Option<Vec<u8>> = conn.query_row(
        "SELECT b.embedding FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        rusqlite::params!["a1", "no_emb"],
        |r| r.get(0),
    )?;
    assert!(blob.is_none(), "embedding must be NULL when embedder is None");
    Ok(())
}

#[test]
fn backfill_embeddings_populates_null_rows() -> Result<()> {
    use crate::sqlite::embedding::{Embedder, backfill_embeddings};
    struct OneEmbedder;
    impl Embedder for OneEmbedder {
        fn embed(&self, _t: &str) -> crate::error::Result<Vec<f32>> {
            Ok(vec![1.0_f32, 2.0])
        }
        fn dimension(&self) -> usize {
            2
        }
    }

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    // Two blocks written via the existing API → embedding is NULL.
    upsert_memory_block(&db, "a1", "k1", "value one", None, None)?;
    upsert_memory_block(&db, "a1", "k2", "value two", None, None)?;

    // Sanity: both NULL before backfill.
    {
        let conn = db.lock();
        let null_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM shared_memory_blocks WHERE embedding IS NULL",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(null_count, 2);
    }

    let processed = backfill_embeddings(&db, &OneEmbedder)?;
    assert_eq!(processed, 2, "backfill must process both NULL rows");

    let conn = db.lock();
    let null_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM shared_memory_blocks WHERE embedding IS NULL",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(null_count, 0, "no NULL embeddings should remain");

    let any_blob: Vec<u8> = conn.query_row(
        "SELECT embedding FROM shared_memory_blocks WHERE embedding IS NOT NULL LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(any_blob.len(), 2 * 4, "expected 2 f32 le bytes = 8");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// WI-SEMANTIC Phase 3: cosine similarity search over stored embeddings
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn search_memory_semantic_ranks_by_cosine_similarity() -> Result<()> {
    use crate::sqlite::embedding::{Embedder, search_memory_semantic};
    use crate::sqlite::memory::upsert_memory_block_with_embedder;

    /// Maps the first byte of the input string to a fixed direction vector,
    /// so the test can build deterministic clusters without fastembed.
    struct LetterEmbedder;
    impl Embedder for LetterEmbedder {
        fn embed(&self, text: &str) -> crate::error::Result<Vec<f32>> {
            // Three orthogonal basis vectors keyed on first char.
            let v = match text.chars().next().unwrap_or('?') {
                'a' => vec![1.0, 0.0, 0.0],
                'b' => vec![0.0, 1.0, 0.0],
                'c' => vec![0.0, 0.0, 1.0],
                _ => vec![0.5, 0.5, 0.5],
            };
            Ok(v)
        }
        fn dimension(&self) -> usize {
            3
        }
    }

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Three blocks, each in its own basis direction.
    upsert_memory_block_with_embedder(&db, "a1", "alpha", "alpha note", None, None, Some(&LetterEmbedder))?;
    upsert_memory_block_with_embedder(&db, "a1", "beta",  "bravo note", None, None, Some(&LetterEmbedder))?;
    upsert_memory_block_with_embedder(&db, "a1", "gamma", "charlie note", None, None, Some(&LetterEmbedder))?;

    // Query "a"-direction — should rank `alpha` first.
    let q = LetterEmbedder.embed("alpha query")?;
    let conn = db.lock();
    let hits = search_memory_semantic(&conn, "a1", &q, None, 10)?;
    assert!(!hits.is_empty(), "semantic search returned no rows");

    // First hit must be the alpha block.
    let (_id, _score, label, _value) = &hits[0];
    assert_eq!(label, "alpha", "expected 'alpha' first, got {hits:?}");
    Ok(())
}

#[test]
fn search_memory_semantic_skips_null_embedding_rows() -> Result<()> {
    use crate::sqlite::embedding::{Embedder, search_memory_semantic};

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    // Block has NO embedding — must be filtered out by semantic search.
    upsert_memory_block(&db, "a1", "no_emb", "value with no embedding", None, None)?;

    struct ZeroEmbedder;
    impl Embedder for ZeroEmbedder {
        fn embed(&self, _text: &str) -> crate::error::Result<Vec<f32>> {
            Ok(vec![1.0, 0.0])
        }
        fn dimension(&self) -> usize {
            2
        }
    }

    let q = ZeroEmbedder.embed("anything")?;
    let conn = db.lock();
    let hits = search_memory_semantic(&conn, "a1", &q, None, 10)?;
    assert!(hits.is_empty(), "rows without an embedding must not appear in semantic results");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// WI-SEMANTIC Phase 3: hybrid search_memory_hybrid (keyword + semantic via RRF)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn search_memory_hybrid_finds_non_keyword_conceptual_match() -> Result<()> {
    use crate::sqlite::embedding::Embedder;
    use crate::sqlite::memory::upsert_memory_block_with_embedder;
    use crate::sqlite::tools::search_memory_hybrid;

    /// Maps any input containing 'lock' OR 'mutex' to direction A,
    /// other inputs to direction B. So a query about deadlocks (no
    /// keyword overlap with 'mutex') still embeds close to direction A.
    struct ConceptualEmbedder;
    impl Embedder for ConceptualEmbedder {
        fn embed(&self, text: &str) -> crate::error::Result<Vec<f32>> {
            let lower = text.to_lowercase();
            let v = if lower.contains("lock")
                || lower.contains("mutex")
                || lower.contains("deadlock")
                || lower.contains("contention")
            {
                vec![1.0_f32, 0.0]
            } else {
                vec![0.0_f32, 1.0]
            };
            Ok(v)
        }
        fn dimension(&self) -> usize {
            2
        }
    }

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block_with_embedder(
        &db,
        "a1",
        "fix_note",
        // Note: the value mentions 'mutex' but NOT 'deadlock'.
        "We scoped the parking_lot mutex to a smaller block.",
        None,
        None,
        Some(&ConceptualEmbedder),
    )?;
    upsert_memory_block_with_embedder(
        &db,
        "a1",
        "unrelated",
        "Some unrelated note about colors",
        None,
        None,
        Some(&ConceptualEmbedder),
    )?;

    // Pure-keyword search for 'deadlock' MUST return zero hits — the value
    // has no keyword overlap. This documents the previous behaviour.
    let kw = crate::sqlite::tools::search_memory(&db, "a1", "deadlock", None)?;
    assert!(
        kw.iter().all(|(l, _, _)| l != "fix_note"),
        "baseline: keyword search must not find 'fix_note' for 'deadlock'; got {kw:?}"
    );

    // Hybrid search WITH the embedder MUST surface 'fix_note' via the
    // semantic path (and via RRF should rank it among the top results).
    let hybrid = search_memory_hybrid(&db, "a1", "deadlock", None, Some(&ConceptualEmbedder))?;
    let labels: Vec<&str> = hybrid.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(
        labels.contains(&"fix_note"),
        "hybrid search must surface 'fix_note' for conceptual query 'deadlock'; got {labels:?}"
    );
    Ok(())
}

#[test]
fn search_memory_hybrid_with_none_embedder_matches_old_behaviour() -> Result<()> {
    use crate::sqlite::embedding::Embedder;
    use crate::sqlite::tools::{search_memory, search_memory_hybrid};

    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "rust_tip", "parking_lot Mutex tips", None, None)?;
    upsert_memory_block(&db, "a1", "other", "irrelevant text", None, None)?;

    let old: Vec<String> = search_memory(&db, "a1", "parking_lot", None)?
        .into_iter()
        .map(|(l, _, _)| l)
        .collect();
    let new: Vec<String> = search_memory_hybrid(&db, "a1", "parking_lot", None, None::<&dyn Embedder>)?
        .into_iter()
        .map(|(l, _, _)| l)
        .collect();
    assert_eq!(old, new, "hybrid with None embedder must equal legacy search_memory");
    Ok(())
}


// ── A1: Write-ahead truncation tests ─────────────────────────────────────────

#[test]
fn test_a1_truncation_returns_warning_not_error() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Write a value that exceeds the max_chars limit
    let long_value = "x".repeat(150);
    let wr = upsert_memory_block(&db, "a1", "small_block", &long_value, None, Some(100))?;

    // Should truncate, not error
    assert!(wr.was_truncated, "expected was_truncated = true");
    assert_eq!(wr.stored_chars, 100);
    assert_eq!(wr.requested_chars, 150);

    // Verify the stored value is actually 100 chars
    let blocks = get_memory_blocks(&db, "a1")?;
    let stored = blocks.iter().find(|(l, _, _)| l == "small_block").unwrap();
    assert_eq!(stored.1.chars().count(), 100);
    Ok(())
}

#[test]
fn test_a1_no_truncation_when_under_limit() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let value = "hello world";
    let wr = upsert_memory_block(&db, "a1", "ok_block", value, None, Some(1000))?;

    assert!(!wr.was_truncated);
    assert_eq!(wr.stored_chars, value.len());
    assert_eq!(wr.requested_chars, value.len());
    Ok(())
}

#[test]
fn test_a1_no_truncation_without_limit() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let long_value = "y".repeat(50_000);
    let wr = upsert_memory_block(&db, "a1", "unlimited", &long_value, None, None)?;

    assert!(!wr.was_truncated);
    assert_eq!(wr.stored_chars, 50_000);
    Ok(())
}

// ── A3: Provenance tests ────────────────────────────────────────────────────

#[test]
fn test_a3_stamp_provenance_sets_columns() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "fact1", "some fact", None, None)?;

    // Stamp provenance
    stamp_provenance(&db, "a1", "fact1", Some(42), None, Some("tc-abc-123"), Some("tc-abc-123"));

    // Verify columns
    let conn = db.lock();
    let (turn, tc_id): (Option<i64>, Option<String>) = conn.query_row(
        "SELECT b.source_turn, b.source_te_id
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'fact1'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    assert_eq!(turn, Some(42));
    assert_eq!(tc_id.as_deref(), Some("tc-abc-123"));
    Ok(())
}

#[test]
fn test_a3_stamp_provenance_coalesce_preserves_existing() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "fact2", "data", None, None)?;

    // First stamp sets both
    stamp_provenance(&db, "a1", "fact2", Some(10), None, Some("tc-first"), Some("tc-first"));
    // Second stamp with only turn — should not overwrite tc_id
    stamp_provenance(&db, "a1", "fact2", Some(20), None, None, None);

    let conn = db.lock();
    let (turn, tc_id): (Option<i64>, Option<String>) = conn.query_row(
        "SELECT b.source_turn, b.source_te_id
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'fact2'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    assert_eq!(turn, Some(20), "turn should be updated to 20");
    assert_eq!(tc_id.as_deref(), Some("tc-first"), "tc_id should be preserved");
    Ok(())
}

#[test]
fn test_a3_stamp_provenance_noop_when_both_none() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;
    upsert_memory_block(&db, "a1", "fact3", "data", None, None)?;

    // Stamp with both None — should be a no-op
    stamp_provenance(&db, "a1", "fact3", None, None, None, None);

    let conn = db.lock();
    let (turn, tc_id): (Option<i64>, Option<String>) = conn.query_row(
        "SELECT b.source_turn, b.source_te_id
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'fact3'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    assert_eq!(turn, None, "source_turn should remain NULL");
    assert_eq!(tc_id, None, "source_te_id should remain NULL");
    Ok(())
}


// ── A5: Semantic chunking tests ─────────────────────────────────────────────

#[test]
fn test_a5_chunk_text_short_returns_single() {
    let text = "Short text under threshold.";
    let chunks = chunk_text(text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].index, 0);
    assert_eq!(chunks[0].content, text);
}

#[test]
fn test_a5_chunk_text_long_splits_at_sentences() {
    // Build a text with clear sentence boundaries, > 500 chars.
    let sentences: Vec<String> = (0..20)
        .map(|i| format!("This is sentence number {}.", i))
        .collect();
    let text = sentences.join(" ");
    assert!(text.chars().count() > CHUNK_THRESHOLD);

    let chunks = chunk_text(&text);
    assert!(chunks.len() >= 2, "long text should produce multiple chunks");

    // All original content should be covered.
    for chunk in &chunks {
        assert!(!chunk.content.is_empty());
        assert!(chunk.content.len() <= CHUNK_TARGET + 200,
            "chunk {} is {} bytes, overly large", chunk.index, chunk.content.len());
    }

    // Last chunk should end at or near the text end.
    let last = &chunks[chunks.len() - 1];
    assert!(text.ends_with(last.content.trim_end()),
        "last chunk should cover the end of text");
}

#[test]
fn test_a5_chunk_text_overlap_exists() {
    let sentences: Vec<String> = (0..20)
        .map(|i| format!("Sentence {}: The quick brown fox jumps.", i))
        .collect();
    let text = sentences.join(" ");

    let chunks = chunk_text(&text);
    if chunks.len() >= 2 {
        // Check that consecutive chunks have overlapping content.
        let c0_end: String = chunks[0].content.chars().rev().take(30).collect::<String>()
            .chars().rev().collect();
        let c1_start: String = chunks[1].content.chars().take(60).collect();
        // The overlap means some substring at the end of chunk 0 appears
        // at the start of chunk 1.
        let overlap_found = c1_start.contains(&c0_end[..c0_end.len().min(20)]);
        assert!(overlap_found, "chunks should overlap by ~{CHUNK_OVERLAP} chars");
    }
}

#[test]
fn test_a5_rechunk_block_stores_chunks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Write a large block.
    let sentences: Vec<String> = (0..20)
        .map(|i| format!("Fact {}: The database uses PostgreSQL for persistence.", i))
        .collect();
    let big_value = sentences.join(" ");
    upsert_memory_block(&db, "a1", "big_block", &big_value, None, None)?;

    // Rechunk it.
    rechunk_block(&db, "a1", "big_block", &big_value, None);

    // Verify chunks exist.
    let conn = db.lock();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_chunks mc
         JOIN shared_memory_blocks b ON b.id = mc.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'big_block'",
        [],
        |r| r.get(0),
    )?;
    assert!(count >= 2, "big block should have multiple chunks, got {count}");
    Ok(())
}

#[test]
fn test_a5_rechunk_block_skips_small_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    upsert_memory_block(&db, "a1", "tiny", "hello", None, None)?;
    rechunk_block(&db, "a1", "tiny", "hello", None);

    let conn = db.lock();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_chunks mc
         JOIN shared_memory_blocks b ON b.id = mc.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'tiny'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 0, "small block should have no chunks");
    Ok(())
}

#[test]
fn test_a5_rechunk_replaces_old_chunks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let big1: String = (0..25).map(|i| format!("Version1 sentence number {} with extra text here. ", i)).collect();
    upsert_memory_block(&db, "a1", "evolving", &big1, None, None)?;
    rechunk_block(&db, "a1", "evolving", &big1, None);

    let conn = db.lock();
    let count1: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_chunks mc
         JOIN shared_memory_blocks b ON b.id = mc.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'evolving'",
        [],
        |r| r.get(0),
    )?;
    drop(conn);

    // Re-write the block with different content.
    let big2: String = (0..25).map(|i| format!("Version2 updated data point number {}. ", i)).collect();
    upsert_memory_block(&db, "a1", "evolving", &big2, None, None)?;
    rechunk_block(&db, "a1", "evolving", &big2, None);

    let conn = db.lock();
    // Verify no leftover Version1 chunks.
    let v1_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_chunks mc
         JOIN shared_memory_blocks b ON b.id = mc.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'evolving'
           AND mc.content LIKE '%Version1%'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(v1_count, 0, "old chunks should be deleted on rechunk");

    let count2: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_chunks mc
         JOIN shared_memory_blocks b ON b.id = mc.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = 'a1' AND b.label = 'evolving'",
        [],
        |r| r.get(0),
    )?;
    assert!(count2 >= 2, "new chunks should exist, got {count2}");
    assert!(count1 > 0, "first chunk pass should have produced chunks");
    Ok(())
}

// ── A6: Chunk-level search tests ─────────────────────────────────────────────

#[test]
fn test_a6_search_memory_finds_chunk_content() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Write a large block with a distinctive keyword buried inside.
    let mut sentences: Vec<String> = (0..18)
        .map(|i| format!("Generic filler sentence number {}.", i))
        .collect();
    sentences.push("The xylophone_secret_key is stored in the vault.".to_string());
    sentences.push("End of block content.".to_string());
    let big_value = sentences.join(" ");

    upsert_memory_block(&db, "a1", "secrets", &big_value, None, None)?;
    rechunk_block(&db, "a1", "secrets", &big_value, None);

    // Search for the distinctive keyword.
    let results = super::search_memory(&db, "a1", "xylophone_secret_key", None)?;
    assert!(!results.is_empty(), "search should find chunk with xylophone_secret_key");

    let found_label = results.iter().any(|(l, _, _)| l == "secrets");
    assert!(found_label, "result should reference the 'secrets' block");
    Ok(())
}


// ── A9: Proactive recall tests ──────────────────────────────────────────────

#[test]
fn test_a9_recall_chunks_finds_keyword_match() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Write a large block with a distinctive keyword in the middle.
    let mut sentences: Vec<String> = (0..18)
        .map(|i| format!("Generic filler sentence number {} for padding. ", i))
        .collect();
    sentences.push("The PostgreSQL database connection pool uses 20 threads. ".to_string());
    sentences.push("End of block content. ".to_string());
    let big_value = sentences.join("");

    upsert_memory_block(&db, "a1", "infra", &big_value, None, None)?;
    rechunk_block(&db, "a1", "infra", &big_value, None);

    // Recall with a query that should match the keyword.
    let results = recall_chunks(&db, "a1", "PostgreSQL connection pool", 3);
    assert!(!results.is_empty(), "should recall chunks matching PostgreSQL");
    assert_eq!(results[0].label, "infra");
    assert!(results[0].chunk_content.contains("PostgreSQL"),
        "recalled chunk should contain the keyword");
    Ok(())
}

#[test]
fn test_a9_recall_chunks_empty_query_returns_nothing() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    let big: String = (0..20).map(|i| format!("Data point {}. ", i)).collect();
    upsert_memory_block(&db, "a1", "stuff", &big, None, None)?;
    rechunk_block(&db, "a1", "stuff", &big, None);

    let results = recall_chunks(&db, "a1", "", 3);
    assert!(results.is_empty(), "empty query should return nothing");

    let results2 = recall_chunks(&db, "a1", "ab cd", 3); // all words < 3 chars
    assert!(results2.is_empty(), "short-word-only query should return nothing");
    Ok(())
}

#[test]
fn test_a9_recall_chunks_deduplicates_by_label() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Block with the keyword repeated in multiple chunks.
    let repeated: String = (0..30)
        .map(|i| format!("Sentence {} mentions the xylophone instrument. ", i))
        .collect();
    upsert_memory_block(&db, "a1", "music", &repeated, None, None)?;
    rechunk_block(&db, "a1", "music", &repeated, None);

    let results = recall_chunks(&db, "a1", "xylophone instrument", 5);
    // Even though multiple chunks match, we should get at most 1 per label.
    let label_count = results.iter().filter(|r| r.label == "music").count();
    assert_eq!(label_count, 1, "should deduplicate to 1 result per label, got {label_count}");
    Ok(())
}


// ── A11: Recency × Frequency scoring tests ──────────────────────────────────

#[test]
fn test_a11_score_fresh_high_access_beats_stale() {
    use super::super::tools::recency_frequency_score;

    // Block A: recently written (turn 100), accessed 5 times. Current turn = 100.
    let score_a = recency_frequency_score(100, 100, 100, 5);

    // Block B: written at turn 10, never accessed. Current turn = 100.
    let score_b = recency_frequency_score(100, 10, 0, 0);

    assert!(score_a > score_b,
        "fresh+frequent ({score_a:.3}) should beat stale+unread ({score_b:.3})");
}

#[test]
fn test_a11_score_frequency_boost_is_logarithmic() {
    use super::super::tools::recency_frequency_score;

    // Same recency (turn 50, current 50), different access counts.
    let score_0 = recency_frequency_score(50, 50, 50, 0);
    let score_10 = recency_frequency_score(50, 50, 50, 10);
    let score_100 = recency_frequency_score(50, 50, 50, 100);

    assert!(score_10 > score_0, "10 accesses should score higher than 0");
    assert!(score_100 > score_10, "100 accesses should score higher than 10");
    // Logarithmic: the gap between 0→10 should be larger than 10→100.
    let delta_0_10 = score_10 - score_0;
    let delta_10_100 = score_100 - score_10;
    assert!(delta_0_10 > delta_10_100 * 0.5,
        "diminishing returns: 0→10 gap ({delta_0_10:.3}) should be significant vs 10→100 ({delta_10_100:.3})");
}

#[test]
fn test_a11_score_recency_decay() {
    use super::super::tools::recency_frequency_score;

    // Same access count (5), different staleness.
    let fresh = recency_frequency_score(100, 100, 100, 5);  // 0 turns idle
    let mid   = recency_frequency_score(100, 50, 50, 5);    // 50 turns idle
    let stale = recency_frequency_score(100, 0, 0, 5);      // 100 turns idle

    assert!(fresh > mid, "fresh ({fresh:.3}) should beat mid-stale ({mid:.3})");
    assert!(mid > stale, "mid-stale ({mid:.3}) should beat very stale ({stale:.3})");
    // At 50 turns idle, recency weight ≈ 0.5.
    assert!(mid < fresh * 0.7, "50-turn-idle block should score significantly less than fresh");
}

#[test]
fn test_a11_search_memory_ranks_frequent_block_higher() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "a1")?;

    // Both blocks contain the keyword "database".
    upsert_memory_block(&db, "a1", "old_db_info", "The database uses PostgreSQL", None, None)?;
    upsert_memory_block(&db, "a1", "new_db_info", "The database connection pool is 20", None, None)?;

    // Bump access on old_db_info many times to boost its frequency.
    for _ in 0..8 {
        bump_block_access(&db, "a1", &["old_db_info"]);
    }

    let results = super::super::tools::search_memory(&db, "a1", "database", None)?;
    assert!(results.len() >= 2, "should find both blocks");

    // The frequently-accessed block should rank first despite being written first.
    assert_eq!(results[0].0, "old_db_info",
        "frequently-accessed block should rank higher");
    Ok(())
}


// ── A15: Subagent write-back tests ──────────────────────────────────────────

#[test]
fn test_a15_write_back_copies_custom_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "parent")?;
    make_agent(&db, "sa_001")?;

    // Subagent writes some facts.
    upsert_memory_block(&db, "sa_001", "api_design", "REST with HATEOAS links", None, None)?;
    upsert_memory_block(&db, "sa_001", "db_choice", "PostgreSQL 16", Some("database decision"), None)?;

    let written = write_back_subagent_memory(&db, "sa_001", "parent");
    assert_eq!(written, 2, "should write back 2 custom blocks");

    // Verify they exist under parent with subagent: prefix.
    let parent_blocks = get_memory_blocks(&db, "parent")?;
    let labels: Vec<&str> = parent_blocks.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(labels.contains(&"subagent:api_design"), "should have subagent:api_design");
    assert!(labels.contains(&"subagent:db_choice"), "should have subagent:db_choice");

    // Verify values were copied correctly.
    let api_val = parent_blocks.iter().find(|(l, _, _)| l == "subagent:api_design")
        .map(|(_, v, _)| v.as_str()).unwrap_or("");
    assert_eq!(api_val, "REST with HATEOAS links");
    Ok(())
}

#[test]
fn test_a15_write_back_excludes_system_blocks() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "parent")?;
    make_agent(&db, "sa_002")?;

    // Subagent has system blocks (seeded from parent) + a custom one.
    upsert_memory_block(&db, "sa_002", "persona", "I am helpful", None, None)?;
    upsert_memory_block(&db, "sa_002", "human", "User is Alice", None, None)?;
    upsert_memory_block(&db, "sa_002", "project", "Rust project", None, None)?;
    upsert_memory_block(&db, "sa_002", "active_goal", "doing stuff", None, None)?;
    upsert_memory_block(&db, "sa_002", "skill:rust", "rust skill body", None, None)?;
    upsert_memory_block(&db, "sa_002", "finding", "discovered bug in auth", None, None)?;

    let written = write_back_subagent_memory(&db, "sa_002", "parent");
    assert_eq!(written, 1, "only 'finding' should be written back");

    let parent_blocks = get_memory_blocks(&db, "parent")?;
    let labels: Vec<&str> = parent_blocks.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(labels.contains(&"subagent:finding"), "should have subagent:finding");
    assert!(!labels.contains(&"subagent:persona"), "should NOT have subagent:persona");
    assert!(!labels.contains(&"subagent:skill:rust"), "should NOT have subagent:skill:rust");
    Ok(())
}

#[test]
fn test_a15_write_back_skips_empty_values() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "parent")?;
    make_agent(&db, "sa_003")?;

    upsert_memory_block(&db, "sa_003", "empty_block", "", None, None)?;
    upsert_memory_block(&db, "sa_003", "whitespace_block", "   \n  ", None, None)?;
    upsert_memory_block(&db, "sa_003", "real_block", "valuable data", None, None)?;

    let written = write_back_subagent_memory(&db, "sa_003", "parent");
    assert_eq!(written, 1, "only non-empty block should be written back");
    Ok(())
}

#[test]
fn test_a15_write_back_no_blocks_returns_zero() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "parent")?;
    make_agent(&db, "sa_004")?;

    let written = write_back_subagent_memory(&db, "sa_004", "parent");
    assert_eq!(written, 0, "no blocks → 0 written back");
    Ok(())
}

/// REC-3: Blocks whose label starts with `subagent:` must NOT be written
/// back.  Without this filter a cascading chain of subagent invocations
/// produces labels like `subagent:subagent:subagent:foo`.
#[test]
fn test_a15_write_back_excludes_subagent_prefix() -> Result<()> {
    let db = setup_mem_db()?;
    make_agent(&db, "parent")?;
    make_agent(&db, "sa_005")?;

    // Simulate a subagent that inherited `subagent:foo` from a previous
    // write-back cycle (the parent seeded it), plus a genuine new finding.
    upsert_memory_block(&db, "sa_005", "subagent:foo", "inherited data", None, None)?;
    upsert_memory_block(&db, "sa_005", "subagent:bar:baz", "deeply inherited", None, None)?;
    upsert_memory_block(&db, "sa_005", "new_finding", "fresh discovery", None, None)?;

    let written = write_back_subagent_memory(&db, "sa_005", "parent");
    assert_eq!(written, 1, "only 'new_finding' should be written back");

    let parent_blocks = get_memory_blocks(&db, "parent")?;
    let labels: Vec<&str> = parent_blocks.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(
        labels.contains(&"subagent:new_finding"),
        "should have subagent:new_finding"
    );
    // These must NOT appear — they would cascade to subagent:subagent:*
    assert!(
        !labels.contains(&"subagent:subagent:foo"),
        "must NOT cascade subagent:subagent:foo"
    );
    assert!(
        !labels.contains(&"subagent:subagent:bar:baz"),
        "must NOT cascade subagent:subagent:bar:baz"
    );
    Ok(())
}

use super::*;
use crate::sqlite::{self, AgentRow, TimelineHorizon};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn agent(db: &Db, id: &str) {
    sqlite::create_agent(
        db,
        &AgentRow {
            id: id.into(),
            name: id.into(),
            model: "m".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();
}

fn message(db: &Db, agent_id: &str, conv: Option<&str>, id: &str, ts: i64) {
    db.get().unwrap().execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5, 10)",
        params![id, agent_id, conv, serde_json::json!({"content": id}).to_string(), ts],
    ).unwrap();
}

fn seeded() -> Db {
    let db = sqlite::open(":memory:").unwrap();
    agent(&db, "a");
    message(&db, "a", None, "dropped", 100);
    message(&db, "a", None, "retained", 100);
    db
}

fn plan(value: &str) -> SummaryPlan {
    SummaryPlan {
        upserts: vec![("session_summary".into(), value.into())],
        ..Default::default()
    }
}

fn ids(rows: Vec<MessageRow>) -> Vec<String> {
    rows.into_iter().map(|m| m.id).collect()
}

#[test]
fn same_timestamp_and_concurrent_inserts_survive_historical_anchor() -> TestResult {
    let db = seeded();
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    assert_eq!(snapshot.messages().len(), 2);
    // A real concurrent writer can use the DB while a captured snapshot is held.
    let writer_db = db.clone();
    std::thread::spawn(move || {
        message(&writer_db, "a", None, "during-llm", 100);
        message(&writer_db, "a", None, "clock-regressed", 99);
    })
    .join()
    .unwrap();
    snapshot.commit(1, 1, &plan("summary"))?;
    let expected = vec!["clock-regressed", "retained", "during-llm"];
    assert_eq!(
        ids(sqlite::list_messages_since_last_compaction(
            &db, "a", None, 100
        )?),
        expected
    );
    assert_eq!(
        ids(sqlite::get_context_window(&db, "a", None, 100_000)?),
        expected
    );
    assert_eq!(
        sqlite::list_messages(&db, "a", None, 100)?.len(),
        4,
        "full history is retained"
    );
    let conn = db.get()?;
    let (ts, anchor_seq, marker_seq): (i64, i64, i64) = conn.query_row(
        "SELECT created_at, compaction_anchor_seq, history_seq FROM messages WHERE role = 'compaction'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(ts, 100);
    assert!(
        anchor_seq < marker_seq,
        "tie-breaker comes from historical source"
    );
    drop(conn);
    // Next pass consumes only the late backdated row; older covered rows stay
    // covered even though the new historical source anchor regresses.
    let next = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    next.commit(1, 1, &plan("summary with late row"))?;
    assert_eq!(
        ids(sqlite::get_context_window(&db, "a", None, 100_000)?),
        vec!["retained", "during-llm"]
    );
    Ok(())
}

#[test]
fn rollback_at_marker_restores_rotation_identity_tiers_history_and_archives() -> TestResult {
    let db = seeded();
    sqlite::upsert_memory_block(&db, "a", "session_summary", "live-before", None, None)?;
    sqlite::upsert_memory_block(&db, "a", "session_summary_1", "slot-before", None, None)?;
    sqlite::upsert_memory_block(&db, "a", "session_summary_8", "evicted-before", None, None)?;
    sqlite::upsert_memory_block(&db, "a", "session_index", "index-before", None, None)?;
    let before = managed_blocks(&*db.get()?, "a")?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let archive =
        snapshot.archive_source("raw retained even after failure", &["dropped-turns".into()])?;
    let rotation = SummaryPlan {
        upserts: vec![
            ("session_summary_1".into(), "live-before".into()),
            ("session_summary_8".into(), "shifted".into()),
            ("session_summary".into(), "live-after".into()),
        ],
        deletes: vec!["session_summary_1".into(), "session_summary_8".into()],
        append_to_index: Some("eviction line".into()),
        archive_content: Some("evicted-before".into()),
    };
    db.get()?.execute_batch(
        "CREATE TRIGGER fail_marker BEFORE INSERT ON messages WHEN NEW.role = 'compaction'
         BEGIN SELECT RAISE(ABORT, 'injected horizon failure'); END;",
    )?;
    assert!(snapshot.commit(1, 1, &rotation).is_err());
    assert_eq!(managed_blocks(&*db.get()?, "a")?, before);
    let conn = db.get()?;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_history", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM archival_memory", [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        conn.query_row("SELECT id FROM archival_memory", [], |r| r
            .get::<_, String>(0))?,
        archive
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE role = 'compaction'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM compaction_coverage", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    conn.execute_batch("DROP TRIGGER fail_marker")?;
    drop(conn);
    // Retrying the same captured plan now succeeds and preserves slot IDs.
    snapshot.commit(1, 1, &rotation)?;
    let after = managed_blocks(&*db.get()?, "a")?;
    for old in before {
        let new = after.iter().find(|b| b.label == old.label).unwrap();
        assert_eq!(new.id, old.id);
    }
    let blocks = sqlite::get_memory_blocks_full(&db, "a")?;
    assert!(
        blocks
            .iter()
            .any(|b| b.0 == "session_summary" && b.3 == "pinned")
    );
    assert!(
        blocks
            .iter()
            .any(|b| b.0 == "session_summary_1" && b.3 == "long")
    );
    assert!(!is_claimed(&db, "a", None)?);
    Ok(())
}

#[test]
fn rollback_on_summary_write_does_not_publish_index_or_evict_slot() -> TestResult {
    let db = seeded();
    sqlite::upsert_memory_block(&db, "a", "session_summary", "before", None, None)?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let before = managed_blocks(&*db.get()?, "a")?;
    db.get()?.execute_batch(
        "CREATE TRIGGER fail_summary BEFORE UPDATE OF value ON shared_memory_blocks
         WHEN NEW.label = 'session_summary' BEGIN SELECT RAISE(ABORT, 'injected summary failure'); END;",
    )?;
    let mut update = plan("after");
    update
        .upserts
        .insert(0, ("session_summary_1".into(), "before".into()));
    update.append_to_index = Some("index".into());
    update.archive_content = Some("evicted".into());
    assert!(snapshot.commit(1, 1, &update).is_err());
    assert_eq!(managed_blocks(&*db.get()?, "a")?, before);
    assert!(!TimelineHorizon::has_compaction_marker(&db, "a", None)?);
    assert_eq!(
        db.get()?
            .query_row("SELECT COUNT(*) FROM archival_memory", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn racing_claims_use_sqlite_fencing_across_pools_and_release_on_drop() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("claims.sqlite");
    let path = path.to_str().unwrap();
    let db1 = sqlite::open(path)?;
    agent(&db1, "a");
    let db2 = sqlite::open(path)?;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [db1.clone(), db2]
        .into_iter()
        .map(|db| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                ConsolidationSnapshot::capture(&db, "a", None).unwrap()
            })
        })
        .collect();
    let snapshots: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(snapshots.iter().filter(|s| s.is_some()).count(), 1);
    assert!(is_claimed(&db1, "a", None)?);
    // Independent scopes remain usable while the first worker awaits its LLM.
    let other = ConsolidationSnapshot::capture(&db1, "a", Some(""))?.unwrap();
    drop(other);
    drop(snapshots);
    assert!(ConsolidationSnapshot::capture(&db1, "a", None)?.is_some());
    Ok(())
}

#[test]
fn expired_claim_is_fenced_and_old_drop_cannot_release_new_owner() -> TestResult {
    let db = seeded();
    let old = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    db.get()?
        .execute("UPDATE consolidation_claims SET expires_at = 0", [])?;
    let new = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    assert!(old.commit(1, 1, &plan("stale summary")).is_err());
    drop(old);
    assert!(is_claimed(&db, "a", None)?);
    new.commit(1, 1, &plan("new owner"))?;
    Ok(())
}

#[test]
fn shared_blocks_conflict_but_same_label_independent_agents_do_not() -> TestResult {
    let db = seeded();
    agent(&db, "b");
    message(&db, "b", None, "b-dropped", 100);
    message(&db, "b", None, "b-retained", 100);
    sqlite::upsert_memory_block(&db, "a", "session_summary", "a-before", None, None)?;
    sqlite::upsert_memory_block(&db, "b", "session_summary", "b-before", None, None)?;
    let a = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let b = ConsolidationSnapshot::capture(&db, "b", None)?.unwrap();
    a.commit(1, 1, &plan("a-after"))?;
    b.commit(1, 1, &plan("b-after"))?;
    sqlite::delete_memory_block(&db, "b", "session_summary")?;
    let a_id = sqlite::get_memory_blocks_with_ids(&db, "a")?
        .into_iter()
        .find(|(_, l, _, _)| l == "session_summary")
        .unwrap()
        .0;
    sqlite::link_shared_memory_block(&db, "b", &a_id)?;
    let a = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let b = ConsolidationSnapshot::capture(&db, "b", None)?.unwrap();
    a.commit(1, 1, &plan("shared-new"))?;
    assert!(b.commit(1, 1, &plan("stale-shared-overwrite")).is_err());
    assert_eq!(
        sqlite::get_block_by_id(&db, &a_id)?.unwrap().value,
        "shared-new"
    );
    assert_eq!(
        sqlite::get_memory_history(&db, "a", "session_summary", 10)?.len(),
        2
    );
    Ok(())
}

#[test]
fn independent_conversation_claims_still_validate_agent_summary_identity() -> TestResult {
    let db = seeded();
    message(&db, "a", Some("named"), "named-dropped", 100);
    sqlite::upsert_memory_block(&db, "a", "session_summary", "before", None, None)?;
    let legacy = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let named = ConsolidationSnapshot::capture(&db, "a", Some("named"))?.unwrap();
    legacy.commit(1, 1, &plan("legacy summary"))?;
    assert!(
        named
            .commit(1, 1, &plan("would lose legacy summary"))
            .is_err()
    );
    assert!(!TimelineHorizon::has_compaction_marker(
        &db,
        "a",
        Some("named")
    )?);
    Ok(())
}

#[test]
fn changed_source_or_relinked_block_rejects_publication() -> TestResult {
    let db = seeded();
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    db.get()?.execute(
        "UPDATE messages SET content = '\"changed\"' WHERE id = 'dropped'",
        [],
    )?;
    assert!(snapshot.commit(1, 1, &plan("obsolete source")).is_err());
    drop(snapshot);
    sqlite::upsert_memory_block(&db, "a", "session_summary", "same value", None, None)?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    sqlite::delete_memory_block(&db, "a", "session_summary")?;
    sqlite::upsert_memory_block(&db, "a", "session_summary", "same value", None, None)?;
    assert!(snapshot.commit(1, 1, &plan("obsolete identity")).is_err());
    assert!(!TimelineHorizon::has_compaction_marker(&db, "a", None)?);
    Ok(())
}

#[test]
fn migration_23_preserves_legacy_history_and_replays_ambiguous_second() -> TestResult {
    let db = seeded();
    message(&db, "a", None, "older", 99);
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO messages (id, agent_id, role, content, created_at) VALUES ('legacy-marker', 'a', 'compaction', 'legacy payload', 100)", [],
    )?;
    conn.execute_batch(
        "DROP TABLE compaction_coverage;
         DROP TABLE compaction_horizons;
         DROP TRIGGER messages_history_sequence;
         DROP TABLE message_history_sequence;
         DROP INDEX idx_messages_history_seq;
         ALTER TABLE messages DROP COLUMN history_seq;
         ALTER TABLE messages DROP COLUMN compaction_anchor_seq;
         ALTER TABLE messages DROP COLUMN compaction_snapshot_seq;
         DROP TABLE consolidation_claims;
         PRAGMA user_version = 22;",
    )?;
    super::super::run_migrations(&conn)?;
    super::super::run_migrations(&conn)?;
    let marker: (String, i64, Option<i64>) = conn.query_row(
        "SELECT content, created_at, compaction_anchor_seq FROM messages WHERE id = 'legacy-marker'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(marker, ("legacy payload".into(), 100, None));
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        24
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE history_seq != rowid OR history_seq IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    drop(conn);
    assert_eq!(
        ids(sqlite::list_messages_since_last_compaction(
            &db, "a", None, 100
        )?),
        vec!["dropped", "retained"]
    );
    assert_eq!(
        ids(sqlite::get_context_window(&db, "a", None, 100_000)?),
        vec!["dropped", "retained"]
    );
    assert_eq!(sqlite::list_messages(&db, "a", None, 100)?.len(), 3);
    assert!(
        sqlite::search_messages(&db, "a", "retained", None)?
            .iter()
            .any(|m| m.id == "retained")
    );
    Ok(())
}

#[test]
fn typed_metadata_follows_shared_identity_not_unrelated_agent_label() -> TestResult {
    let db = seeded();
    agent(&db, "b");
    agent(&db, "c");
    sqlite::upsert_memory_block(&db, "a", "decision", "first", None, None)?;
    sqlite::upsert_memory_block(&db, "b", "decision", "independent", None, None)?;
    let id = sqlite::get_memory_blocks_with_ids(&db, "a")?
        .into_iter()
        .find(|(_, l, _, _)| l == "decision")
        .unwrap()
        .0;
    sqlite::link_shared_memory_block(&db, "c", &id)?;
    sqlite::upsert_memory_block_typed(
        &db,
        "a",
        "decision",
        "chosen approach",
        None,
        None,
        Some("decision"),
        Some(1.7),
    )?;
    let a = sqlite::get_memory_blocks_with_provenance(&db, "a")?;
    let b = sqlite::get_memory_blocks_with_provenance(&db, "b")?;
    let c = sqlite::get_memory_blocks_with_provenance(&db, "c")?;
    assert_eq!(a, c);
    assert_eq!(a[0].3, "decision");
    assert_eq!(a[0].4, 1.7);
    assert_eq!(b[0].1, "independent");
    assert_eq!(b[0].4, 1.0);
    assert_ne!(b[0].3, "decision");
    Ok(())
}

#[test]
fn latest_user_message_extracts_object_text_and_keeps_string_compatibility() -> TestResult {
    let db = seeded();
    assert_eq!(
        sqlite::get_latest_user_message(&db, "a", None)?,
        Some("retained".into())
    );
    sqlite::insert_message(
        &db,
        &MessageRow {
            id: "legacy-string".into(),
            agent_id: "a".into(),
            conversation_id: None,
            role: "user".into(),
            content: serde_json::json!("legacy text"),
            char_count: 11,
        },
    )?;
    assert_eq!(
        sqlite::get_latest_user_message(&db, "a", None)?,
        Some("legacy text".into())
    );
    assert_eq!(
        sqlite::get_latest_user_message(&db, "a", Some("absent"))?,
        None
    );
    Ok(())
}

#[test]
fn typed_metadata_failure_rolls_back_value_and_revision() -> TestResult {
    let db = seeded();
    sqlite::upsert_memory_block(&db, "a", "fact", "before", None, None)?;
    let conn = db.get()?;
    conn.execute_batch(
        "CREATE TRIGGER fail_type BEFORE UPDATE OF memory_type ON shared_memory_blocks
         WHEN NEW.label = 'fact' BEGIN SELECT RAISE(ABORT, 'injected metadata failure'); END;",
    )?;
    drop(conn);
    assert!(
        sqlite::upsert_memory_block_typed(
            &db,
            "a",
            "fact",
            "after",
            None,
            None,
            Some("decision"),
            Some(1.5),
        )
        .is_err()
    );
    let fact = sqlite::get_memory_blocks_with_provenance(&db, "a")?
        .into_iter()
        .find(|b| b.0 == "fact")
        .unwrap();
    assert_eq!(fact.1, "before");
    assert_eq!(fact.4, 1.0);
    assert!(sqlite::get_memory_history(&db, "a", "fact", 10)?.is_empty());
    Ok(())
}

#[test]
fn migration_23_failure_rolls_back_schema_and_version() -> TestResult {
    let db = seeded();
    let conn = db.get()?;
    // Deliberately leave the second new column present, so migration fails
    // after its first ALTER. Neither that first ALTER nor version may survive.
    conn.execute_batch(
        "ALTER TABLE messages DROP COLUMN compaction_anchor_seq;
         DROP TABLE consolidation_claims;
         PRAGMA user_version = 22;",
    )?;
    assert!(super::super::run_migrations(&conn).is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        22
    );
    let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
    let columns = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert!(!columns.iter().any(|c| c == "compaction_anchor_seq"));
    Ok(())
}

#[test]
fn horizon_rejects_missing_or_other_conversation_anchor() -> TestResult {
    let db = seeded();
    assert!(TimelineHorizon::advance(&db, "a", None, "missing", 1).is_err());
    assert!(TimelineHorizon::advance(&db, "a", Some("other"), "dropped", 1).is_err());
    assert!(!TimelineHorizon::has_compaction_marker(&db, "a", None)?);
    Ok(())
}

#[test]
fn deleted_tail_rowid_reuse_and_vacuum_do_not_invalidate_captured_horizon() -> TestResult {
    let db = seeded();
    // Another conversation's tail raises the captured insertion bound.
    message(&db, "a", Some("other"), "deleted-tail", 100);
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    db.get()?
        .execute("DELETE FROM messages WHERE id = 'deleted-tail'", [])?;
    db.get()?.execute_batch("VACUUM")?;
    message(&db, "a", None, "late-backdated", 99);
    let conn = db.get()?;
    let (physical, sequence): (i64, i64) = conn.query_row(
        "SELECT rowid, history_seq FROM messages WHERE id = 'late-backdated'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    assert!(
        physical <= snapshot.snapshot_seq,
        "fixture must reuse an old physical rowid"
    );
    assert!(
        sequence > snapshot.snapshot_seq,
        "the stable insertion order is never reused"
    );
    drop(conn);
    snapshot.commit(1, 1, &plan("summary"))?;
    assert_eq!(
        ids(sqlite::get_context_window(&db, "a", None, 100_000)?),
        vec!["late-backdated", "retained"]
    );
    Ok(())
}

#[test]
fn tool_output_pruning_respects_the_inflight_consolidation_claim() -> TestResult {
    let db = seeded();
    sqlite::insert_message(
        &db,
        &MessageRow {
            id: "large-tool".into(),
            agent_id: "a".into(),
            conversation_id: None,
            role: "tool".into(),
            content: serde_json::json!({"content": "x".repeat(500), "tool_call_id": "call"}),
            char_count: 500,
        },
    )?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    assert_eq!(sqlite::compact_old_tool_outputs(&db, "a", None, 0, 200)?, 0);
    assert_eq!(
        sqlite::list_messages(&db, "a", None, 10)?
            .iter()
            .find(|m| m.id == "large-tool")
            .unwrap()
            .char_count,
        500
    );
    drop(snapshot);
    assert_eq!(sqlite::compact_old_tool_outputs(&db, "a", None, 0, 200)?, 1);
    Ok(())
}

fn assert_visible_prefix(db: &Db, conv: Option<&str>, expected: &[&str]) -> TestResult {
    for _ in 0..3 {
        assert_eq!(
            ids(sqlite::list_messages_since_last_compaction(
                db, "a", conv, 100
            )?),
            expected,
        );
        assert_eq!(
            ids(sqlite::get_context_window(db, "a", conv, 100_000)?),
            expected
        );
        let snapshot = ConsolidationSnapshot::capture(db, "a", conv)?.unwrap();
        assert_eq!(
            snapshot
                .messages()
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    Ok(())
}

#[test]
fn exact_prefix_after_100_anchor_then_a98_b99_preserves_b_across_vacuum_and_repeated_calls()
-> TestResult {
    for conv in [None, Some("")] {
        let db = sqlite::open(":memory:")?;
        agent(&db, "a");
        message(&db, "a", conv, "first100", 100);
        message(&db, "a", conv, "retained100", 100);
        let first = ConsolidationSnapshot::capture(&db, "a", conv)?.unwrap();
        first.commit(1, 1, &plan("first summary"))?;
        let first_marker = TimelineHorizon::marker_id(&db, "a", conv)?;

        // Put a physical rowid hole before A/B, outside the captured scope.
        message(&db, "a", Some("other"), "hole", 100);
        message(&db, "a", conv, "A98", 98);
        message(&db, "a", conv, "B99", 99);
        let second = ConsolidationSnapshot::capture(&db, "a", conv)?.unwrap();
        assert_eq!(
            second
                .messages()
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["A98", "B99", "retained100"]
        );
        let b_sequence: i64 = db.get()?.query_row(
            "SELECT history_seq FROM messages WHERE id = 'B99'",
            [],
            |r| r.get(0),
        )?;
        db.get()?
            .execute("DELETE FROM messages WHERE id = 'hole'", [])?;
        db.get()?.execute_batch("VACUUM")?;
        assert_eq!(
            db.get()?.query_row(
                "SELECT history_seq FROM messages WHERE id = 'B99'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            b_sequence
        );
        // Another clock-regressed insertion during LLM work must survive too.
        message(&db, "a", conv, "during97", 97);
        second.commit(1, 1, &plan("summary through A only"))?;
        let second_marker = TimelineHorizon::marker_id(&db, "a", conv)?;
        assert_ne!(
            first_marker, second_marker,
            "publication revision follows insertion, not source timestamp"
        );
        let (ts, sequence): (i64, i64) = db.get()?.query_row(
            "SELECT created_at, compaction_anchor_seq FROM messages WHERE id = ?1",
            [second_marker.as_deref().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        assert_eq!(
            ts, 98,
            "anchor is the source actually summarized, not max(previous, source)"
        );
        assert_eq!(sequence, second.sources[0].anchor.sequence);
        assert_visible_prefix(&db, conv, &["during97", "B99", "retained100"])?;

        // Repeat a regressed pass without consuming B, then explicitly cover B.
        let third = ConsolidationSnapshot::capture(&db, "a", conv)?.unwrap();
        third.commit(1, 1, &plan("summary through during97"))?;
        assert_visible_prefix(&db, conv, &["B99", "retained100"])?;
        db.get()?.execute_batch("VACUUM")?;
        assert_visible_prefix(&db, conv, &["B99", "retained100"])?;
        let fourth = ConsolidationSnapshot::capture(&db, "a", conv)?.unwrap();
        fourth.commit(1, 1, &plan("now B is summarized"))?;
        assert_visible_prefix(&db, conv, &["retained100"])?;
        assert_eq!(
            sqlite::list_messages(&db, "a", conv, 100)?.len(),
            5,
            "full history remains intact"
        );
    }
    Ok(())
}

#[test]
fn migration_24_freezes_v23_coverage_and_preserves_backdated_retained_prefix() -> TestResult {
    let db = seeded();
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO messages (id, agent_id, role, content, created_at, compaction_anchor_seq, compaction_snapshot_seq)
         VALUES ('v23-marker', 'a', 'compaction', 'original payload', 100, 1, 2)", [],
    )?;
    conn.execute_batch(
        "DROP TABLE compaction_coverage; DROP TABLE compaction_horizons; PRAGMA user_version = 23;",
    )?;
    drop(conn);
    message(&db, "a", None, "A98", 98);
    message(&db, "a", None, "B99", 99);
    let conn = db.get()?;
    super::super::run_migrations(&conn)?;
    super::super::run_migrations(&conn)?;
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        24
    );
    assert_eq!(
        conn.query_row(
            "SELECT content FROM messages WHERE id = 'v23-marker'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "original payload"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM compaction_coverage", [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    drop(conn);
    assert_visible_prefix(&db, None, &["A98", "B99", "retained"])?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    snapshot.commit(1, 1, &plan("only A"))?;
    assert_visible_prefix(&db, None, &["B99", "retained"])?;
    // Future backdated rows must not be pulled into the migrated old boundary.
    message(&db, "a", None, "later96", 96);
    assert_visible_prefix(&db, None, &["later96", "B99", "retained"])?;
    Ok(())
}

#[test]
fn migration_24_failure_rolls_back_schema_version_and_existing_visibility() -> TestResult {
    let db = seeded();
    let conn = db.get()?;
    conn.execute_batch(
        "DROP TABLE compaction_coverage; DROP TABLE compaction_horizons;
         CREATE TABLE compaction_horizons (marker_id TEXT PRIMARY KEY);
         PRAGMA user_version = 23;",
    )?;
    // The first CREATE must roll back when the second CREATE fails.
    assert!(super::super::run_migrations(&conn).is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        23
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'compaction_coverage'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))?,
        2
    );
    conn.execute_batch("DROP TABLE compaction_horizons")?;
    super::super::run_migrations(&conn)?;
    drop(conn);
    assert_visible_prefix(&db, None, &["dropped", "retained"])?;
    Ok(())
}

#[test]
fn migration_24_replays_only_ambiguous_interval_already_hidden_by_v23_regression() -> TestResult {
    let db = seeded();
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO messages (id, agent_id, role, content, created_at, compaction_anchor_seq, compaction_snapshot_seq)
         VALUES ('v23-first', 'a', 'compaction', 'first payload', 100, 1, 2)", [],
    )?;
    drop(conn);
    message(&db, "a", None, "A98", 98);
    message(&db, "a", None, "B99", 99);
    message(&db, "a", Some("other"), "other97", 97);
    let conn = db.get()?;
    let bound: i64 = conn.query_row(
        "SELECT value FROM message_history_sequence WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    // Reproduce the published v23 defect: A was summarized, but p.max retained
    // anchor (100, 1) and widened the snapshot bound so B was hidden too.
    conn.execute(
        "INSERT INTO messages (id, agent_id, role, content, created_at, compaction_anchor_seq, compaction_snapshot_seq)
         VALUES ('v23-broken', 'a', 'compaction', 'only A was summarized', 100, 1, ?1)", [bound],
    )?;
    conn.execute_batch(
        "DROP TABLE compaction_coverage; DROP TABLE compaction_horizons; PRAGMA user_version = 23;",
    )?;
    super::super::run_migrations(&conn)?;
    // Only the old proven prefix stays covered. A/B's exact historical dropped
    // set was never persisted, so this specific ambiguous interval is replayed.
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM compaction_coverage", [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT content FROM messages WHERE id = 'v23-broken'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "only A was summarized"
    );
    drop(conn);
    assert_visible_prefix(&db, None, &["A98", "B99", "retained"])?;
    assert_eq!(
        ids(sqlite::get_context_window(
            &db,
            "a",
            Some("other"),
            100_000
        )?),
        vec!["other97"]
    );
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    snapshot.commit(1, 1, &plan("only A, exactly this time"))?;
    assert_visible_prefix(&db, None, &["B99", "retained"])?;
    Ok(())
}

#[test]
fn coverage_write_failure_rolls_back_summary_and_horizon() -> TestResult {
    let db = seeded();
    sqlite::upsert_memory_block(&db, "a", "session_summary", "before", None, None)?;
    let snapshot = ConsolidationSnapshot::capture(&db, "a", None)?.unwrap();
    let before = managed_blocks(&*db.get()?, "a")?;
    db.get()?.execute_batch(
        "CREATE TRIGGER fail_coverage BEFORE INSERT ON compaction_coverage
         BEGIN SELECT RAISE(ABORT, 'injected coverage failure'); END;",
    )?;
    assert!(snapshot.commit(1, 1, &plan("after")).is_err());
    assert_eq!(managed_blocks(&*db.get()?, "a")?, before);
    assert!(!TimelineHorizon::has_compaction_marker(&db, "a", None)?);
    assert_eq!(
        db.get()?
            .query_row("SELECT COUNT(*) FROM compaction_coverage", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    drop(snapshot);
    assert_visible_prefix(&db, None, &["dropped", "retained"])?;
    Ok(())
}

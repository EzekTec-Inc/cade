//! Regressions for inherited filesystem authority and cancelled approval leases.

use super::*;
use cade_core::permissions::PermissionService;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

fn paths(values: &[&str]) -> Option<Vec<String>> {
    Some(values.iter().map(|value| (*value).to_string()).collect())
}

#[test]
fn inherited_grants_reject_dotdot_and_false_component_prefixes_before_rebase() {
    let primary = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    for root in [primary.path(), clone.path()] {
        for dir in ["src", "private", "src-extra"] {
            std::fs::create_dir(root.join(dir)).unwrap();
        }
    }
    for execution in [primary.path(), clone.path()] {
        for requested in [
            "src/../private",
            "src/not-created/../../private",
            "src/not-created/deeper/../../../private",
            "src-extra",
            "src/../src-extra",
        ] {
            assert_eq!(
                inherited_child_paths(
                    paths(&["src"]),
                    paths(&[requested]),
                    primary.path(),
                    execution
                ),
                Some(vec![]),
                "parent src cannot grant {requested} in {}",
                execution.display(),
            );
        }
    }
}

#[test]
fn inherited_grants_normalize_narrowing_and_keep_empty_authority_denied() {
    let primary = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    for root in [primary.path(), clone.path()] {
        std::fs::create_dir_all(root.join("src/child")).unwrap();
    }
    let expected = clone.path().canonicalize().unwrap().join("src/child");
    assert_eq!(
        inherited_child_paths(
            paths(&["src"]),
            paths(&["src/unused/../child", "src/child"]),
            primary.path(),
            clone.path()
        ),
        Some(vec![expected.to_string_lossy().into_owned()]),
    );
    assert_eq!(
        inherited_child_paths(
            paths(&["src/child"]),
            paths(&["src"]),
            primary.path(),
            clone.path()
        ),
        Some(vec![expected.to_string_lossy().into_owned()]),
    );
    assert_eq!(
        inherited_child_paths(
            paths(&["src"]),
            paths(&["src/future/deep"]),
            primary.path(),
            clone.path()
        ),
        Some(vec![
            clone
                .path()
                .canonicalize()
                .unwrap()
                .join("src/future/deep")
                .to_string_lossy()
                .into_owned()
        ]),
    );
    for (parent, requested) in [
        (paths(&[]), None),
        (None, paths(&[])),
        (paths(&[]), paths(&["src"])),
        (paths(&["src"]), paths(&[])),
        (paths(&[""]), paths(&["src"])),
        (paths(&["src"]), paths(&[" "])),
    ] {
        assert_eq!(
            inherited_child_paths(parent, requested, primary.path(), clone.path()),
            Some(vec![])
        );
    }
    assert_eq!(
        inherited_child_paths(None, None, primary.path(), clone.path()),
        None
    );
}

#[cfg(unix)]
#[test]
fn inherited_grants_resolve_existing_symlinks_and_missing_descendants_without_widening() {
    let primary = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    std::fs::create_dir(primary.path().join("src")).unwrap();
    std::fs::create_dir(primary.path().join("private")).unwrap();
    std::fs::create_dir(primary.path().join("private/deep")).unwrap();
    std::fs::create_dir(clone.path().join("src")).unwrap();
    std::os::unix::fs::symlink("../private", primary.path().join("src/link")).unwrap();
    std::os::unix::fs::symlink("../private/not-created", primary.path().join("src/broken"))
        .unwrap();
    std::os::unix::fs::symlink("../private/deep", primary.path().join("src/deep-link")).unwrap();
    for requested in [
        "src/link",
        "src/link/future/deep",
        "src/link/future/../secret",
        "src/link/../private",
        "src/broken/future",
        "src/deep-link/..",
        "src/deep-link/../future",
    ] {
        assert_eq!(
            inherited_child_paths(
                paths(&["src"]),
                paths(&[requested]),
                primary.path(),
                clone.path()
            ),
            Some(vec![]),
            "symlink request {requested} must not acquire private authority",
        );
    }
    // Each parent grant is checked independently: private is accessible only
    // when that physical subtree was also explicitly granted by the parent.
    assert_eq!(
        inherited_child_paths(
            paths(&["src", "private"]),
            paths(&["src/link/future"]),
            primary.path(),
            clone.path()
        ),
        Some(vec![
            clone
                .path()
                .canonicalize()
                .unwrap()
                .join("private/future")
                .to_string_lossy()
                .into_owned()
        ]),
    );
}

#[cfg(unix)]
#[test]
fn inherited_grants_do_not_rebase_source_authority_through_a_clone_symlink() {
    let primary = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    std::fs::create_dir(primary.path().join("src")).unwrap();
    std::fs::create_dir(clone.path().join("private")).unwrap();
    std::os::unix::fs::symlink("private", clone.path().join("src")).unwrap();
    assert_eq!(
        inherited_child_paths(
            paths(&["src"]),
            paths(&["src"]),
            primary.path(),
            clone.path()
        ),
        Some(vec![])
    );
}

#[cfg(unix)]
#[test]
fn inherited_grants_compare_canonical_parent_aliases_before_projecting_into_a_clone() {
    let primary = tempfile::tempdir().unwrap();
    let alias_root = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(primary.path().join("src/child")).unwrap();
    std::fs::create_dir_all(clone.path().join("src/child")).unwrap();
    let alias = alias_root.path().join("primary");
    std::os::unix::fs::symlink(primary.path(), &alias).unwrap();
    assert_eq!(
        inherited_child_paths(
            Some(vec![
                primary.path().join("src").to_string_lossy().into_owned()
            ]),
            paths(&["src/child"]),
            &alias,
            clone.path(),
        ),
        Some(vec![
            clone
                .path()
                .canonicalize()
                .unwrap()
                .join("src/child")
                .to_string_lossy()
                .into_owned()
        ]),
    );
}

#[tokio::test]
async fn inherited_grants_empty_intersection_denies_native_and_shared_restricted_session_gates() {
    use cade_agent::subagents::{
        SubagentConfig, SubagentLlmExecutor, SubagentMessage, SubagentSession, SubagentToolCall,
        SubagentToolExecutor, SubagentToolPolicy, SubagentTools, SubagentTurnResponse,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct WriteThenBlocked(AtomicUsize);
    #[async_trait::async_trait]
    impl SubagentLlmExecutor for WriteThenBlocked {
        async fn complete_turn(
            &self,
            _: &str,
            _: &str,
            messages: &[SubagentMessage],
            _: &[Value],
        ) -> Result<SubagentTurnResponse, String> {
            let first = self.0.fetch_add(1, Ordering::SeqCst) == 0;
            if !first {
                assert!(
                    messages.iter().any(|message| message.role == "tool"
                        && message.content.contains("outside the allowed paths")),
                    "the shared gate must reject before native execution or approval"
                );
            }
            Ok(SubagentTurnResponse {
                content: None,
                tokens_used: 1,
                tool_calls: vec![if first {
                    SubagentToolCall {
                        id: "write".into(),
                        name: "write_file".into(),
                        arguments: json!({"path":"private/file", "content":"forbidden"}),
                    }
                } else {
                    SubagentToolCall {
                        id: "finish".into(),
                        name: "finish".into(),
                        arguments: json!({"status":"blocked", "summary":"path rejected"}),
                    }
                }],
            })
        }
    }
    struct NativeProbe {
        runtime: cade_agent::tools::runtime::ToolRuntime,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl SubagentToolExecutor for NativeProbe {
        async fn execute_tool(
            &self,
            id: &str,
            name: &str,
            args: &Value,
            _: &Path,
        ) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self.runtime.execute(id.into(), name, args).await.unwrap();
            if result.is_error {
                Err(result.output)
            } else {
                Ok(result.output)
            }
        }
    }
    let primary = tempfile::tempdir().unwrap();
    let clone = tempfile::tempdir().unwrap();
    for root in [primary.path(), clone.path()] {
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::create_dir(root.join("private")).unwrap();
        std::fs::write(root.join("private/file"), "preserved").unwrap();
    }
    let allowed = inherited_child_paths(
        paths(&["src"]),
        paths(&["src/../private"]),
        primary.path(),
        clone.path(),
    );
    let storage = Arc::new(
        cade_agent::agent::HttpTransport::new("http://localhost:0".into(), "unused".into())
            .unwrap(),
    );
    let mut runtime = cade_agent::tools::runtime::ToolRuntime::new(
        storage,
        Arc::new(cade_agent::mcp::McpManager::empty()),
        "grant-review-child".into(),
        clone.path().to_path_buf(),
    );
    runtime.allowed_paths = allowed.clone();
    let rejected = runtime
        .execute(
            "native".into(),
            "write_file",
            &json!({"path":"private/file", "content":"forbidden"}),
        )
        .await
        .unwrap();
    assert!(
        rejected.is_error,
        "native gate must reject an empty intersection"
    );
    let tools = NativeProbe {
        runtime,
        calls: AtomicUsize::new(0),
    };
    let mut session = SubagentSession::new(
        SubagentConfig::from_args(&json!({"prompt":"check grants"})),
        "parent",
    )
    .with_tool_policy(SubagentToolPolicy {
        permissions: cade_core::permissions::PermissionManager::new(
            cade_core::permissions::PermissionMode::BypassPermissions,
        ),
        tools: inherited_child_tools(
            SubagentTools::Restricted {
                allowed_tools: vec!["write_file".into()],
                allowed_paths: vec!["src/../private".into()],
            },
            allowed.as_deref(),
        ),
        inherited_tools: vec!["write_file".into()],
        allow_nesting: false,
        max_depth: 1,
    });
    let outcome = session
        .run_autonomous_loop(
            &WriteThenBlocked(AtomicUsize::new(0)),
            &tools,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            clone.path(),
        )
        .await;
    assert_eq!(outcome.summary_text(), "path rejected");
    assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
    for root in [primary.path(), clone.path()] {
        assert_eq!(
            std::fs::read_to_string(root.join("private/file")).unwrap(),
            "preserved"
        );
    }
}

fn approval_fixture() -> (cade_store::sqlite::Db, String) {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let parent = format!("approval-review-{}", uuid::Uuid::new_v4());
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: parent.clone(),
            name: parent.clone(),
            model: "test".into(),
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
    (db, parent)
}

async fn scoped_event(
    events: &mut tokio::sync::broadcast::Receiver<Value>,
    parent: &str,
) -> cade_api_types::StreamEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await {
                Ok(event) if event["agent_id"].as_str() == Some(parent) => {
                    return serde_json::from_value(event).unwrap();
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("global approval stream closed: {error}"),
            }
        }
    })
    .await
    .expect("scoped approval event must be delivered")
}

fn start_approval(
    db: &cade_store::sqlite::Db,
    parent: &str,
    child: &str,
) -> tokio::task::JoinHandle<Result<bool, String>> {
    let adapter = HeadlessQueueAdapter {
        db: db.clone(),
        parent_agent_id: parent.into(),
        subagent_id: child.into(),
    };
    tokio::spawn(async move {
        adapter
            .request_permission(
                "write_file",
                &json!({"path":"src/file.rs", "content":"reviewed"}),
            )
            .await
    })
}

#[tokio::test]
async fn cancelled_child_approval_resolves_globally_once_before_the_next_request() {
    let (db, parent) = approval_fixture();
    let mut events = crate::server::api::agents::GLOBAL_EVENTS_TX.subscribe();
    let first = start_approval(&db, &parent, "cancelled-child");
    let required = scoped_event(&mut events, &parent).await;
    assert_eq!(required.msg_type(), "approval_required");
    let first_id = required.approval_id().unwrap().to_owned();
    let mut client_pending = BTreeSet::from([first_id.clone()]);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    // Do not wait for cleanup here: admission of a later request must itself
    // order the old resolution before exposing a new actionable modal.
    let second = start_approval(&db, &parent, "later-child");
    let resolved = scoped_event(&mut events, &parent).await;
    assert_eq!(resolved.msg_type(), "approval_resolved");
    assert_eq!(resolved.approval_id(), Some(first_id.as_str()));
    assert_eq!(resolved.data["subagent_id"], "cancelled-child");
    assert_eq!(resolved.data["approved"], false);
    assert!(
        resolved.data["status"]
            .as_str()
            .unwrap()
            .starts_with("denied:")
    );
    client_pending.remove(resolved.approval_id().unwrap());
    assert!(client_pending.is_empty(), "the cached first modal is gone");
    let later = scoped_event(&mut events, &parent).await;
    assert_eq!(later.msg_type(), "approval_required");
    let second_id = later.approval_id().unwrap().to_owned();
    assert_ne!(second_id, first_id);
    assert!(resolved.data["seq"].as_i64().unwrap() < later.data["seq"].as_i64().unwrap());
    client_pending.insert(second_id.clone());
    assert!(!client_pending.contains(&first_id));
    let pending = cade_store::sqlite::list_pending_approvals(&db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, second_id);
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    let resolved_second = scoped_event(&mut events, &parent).await;
    assert_eq!(resolved_second.msg_type(), "approval_resolved");
    assert_eq!(resolved_second.approval_id(), Some(second_id.as_str()));
    client_pending.remove(resolved_second.approval_id().unwrap());
    assert!(client_pending.is_empty());
    assert!(
        cade_store::sqlite::list_pending_approvals(&db)
            .unwrap()
            .is_empty()
    );
    let durable = cade_store::sqlite::global_events_after(&db, 0).unwrap();
    for id in [&first_id, &second_id] {
        assert_eq!(
            durable
                .iter()
                .filter(|(_, kind, payload)| {
                    kind == "approval_resolved"
                        && serde_json::from_str::<Value>(payload).unwrap()["id"].as_str()
                            == Some(id.as_str())
                })
                .count(),
            1,
            "each withdrawal publishes one durable resolution"
        );
    }
}

#[tokio::test]
async fn child_approval_withdrawal_does_not_overwrite_or_duplicate_a_human_resolution() {
    let (db, parent) = approval_fixture();
    let mut events = crate::server::api::agents::GLOBAL_EVENTS_TX.subscribe();
    let waiting = start_approval(&db, &parent, "human-approved-child");
    let required = scoped_event(&mut events, &parent).await;
    let id = required.approval_id().unwrap().to_owned();
    assert!(cade_store::sqlite::resolve_pending_approval(&db, &id, "approved").unwrap());
    crate::server::api::agents::publish_global_event(
        Some(&db),
        "approval_resolved",
        json!({
            "id": id, "status":"approved", "agent_id":parent, "subagent_id":"human-approved-child",
        }),
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(3), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    finish_prior_child_approval_withdrawals(&parent)
        .await
        .unwrap();
    assert_eq!(
        cade_store::sqlite::get_approval_status(&db, &id)
            .unwrap()
            .as_deref(),
        Some("approved")
    );
    assert_eq!(
        cade_store::sqlite::global_events_after(&db, 0)
            .unwrap()
            .iter()
            .filter(|(_, kind, payload)| {
                kind == "approval_resolved"
                    && serde_json::from_str::<Value>(payload).unwrap()["id"].as_str()
                        == Some(id.as_str())
            })
            .count(),
        1
    );
}

#[test]
fn child_approval_drop_offloads_io_without_needing_a_tokio_runtime() {
    let (db, parent) = approval_fixture();
    let id = format!("no-runtime-{}", uuid::Uuid::new_v4());
    cade_store::sqlite::create_pending_approval(
        &db,
        &id,
        &parent,
        Some("shutdown-child"),
        "write_file",
        "{}",
    )
    .unwrap();
    drop(ChildApprovalCleanupGuard {
        db: db.clone(),
        id: id.clone(),
        parent_agent_id: parent.clone(),
        subagent_id: "shutdown-child".into(),
        conversation_id: Some("conversation-at-cancellation".into()),
        armed: true,
    });
    futures::executor::block_on(finish_prior_child_approval_withdrawals(&parent)).unwrap();
    assert!(
        cade_store::sqlite::get_approval_status(&db, &id)
            .unwrap()
            .unwrap()
            .starts_with("denied:")
    );
    let durable = cade_store::sqlite::global_events_after(&db, 0).unwrap();
    let (_, kind, payload) = durable
        .iter()
        .find(|(_, kind, _)| kind == "approval_resolved")
        .unwrap();
    assert_eq!(kind, "approval_resolved");
    let event: cade_api_types::StreamEvent = serde_json::from_str(payload).unwrap();
    assert_eq!(event.approval_id(), Some(id.as_str()));
    assert_eq!(
        event.conversation_id(),
        Some("conversation-at-cancellation")
    );
    assert_eq!(event.data["agent_id"], parent);
    assert_eq!(event.data["subagent_id"], "shutdown-child");
}

#[test]
fn child_approval_drop_does_not_wait_for_an_exhausted_database_pool() {
    let (db, parent) = approval_fixture();
    let id = format!("blocked-pool-{}", uuid::Uuid::new_v4());
    cade_store::sqlite::create_pending_approval(
        &db,
        &id,
        &parent,
        Some("blocked-child"),
        "write_file",
        "{}",
    )
    .unwrap();
    // The in-memory fixture has one connection. Holding it would make a
    // synchronous destructor wait for the pool's 30-second checkout timeout.
    let held = db.get().unwrap();
    let guard = ChildApprovalCleanupGuard {
        db: db.clone(),
        id: id.clone(),
        parent_agent_id: parent.clone(),
        subagent_id: "blocked-child".into(),
        conversation_id: None,
        armed: true,
    };
    let (released, release_seen) = std::sync::mpsc::channel();
    let dropping = std::thread::spawn(move || {
        drop(guard);
        released.send(()).unwrap();
    });
    let returned_without_io = release_seen.recv_timeout(Duration::from_secs(2)).is_ok();
    drop(held);
    dropping.join().unwrap();
    assert!(
        returned_without_io,
        "Drop must return while the database remains unavailable"
    );
    futures::executor::block_on(finish_prior_child_approval_withdrawals(&parent)).unwrap();
    assert!(
        cade_store::sqlite::get_approval_status(&db, &id)
            .unwrap()
            .unwrap()
            .starts_with("denied:")
    );
}

#[tokio::test]
async fn child_approval_withdrawal_failure_is_observed_before_a_later_request_is_exposed() {
    let (db, parent) = approval_fixture();
    let id = format!("withdrawal-failure-{}", uuid::Uuid::new_v4());
    cade_store::sqlite::create_pending_approval(
        &db,
        &id,
        &parent,
        Some("failed-child"),
        "write_file",
        "{}",
    )
    .unwrap();
    db.get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_withdrawal BEFORE UPDATE OF status ON pending_approvals
         BEGIN SELECT RAISE(FAIL, 'injected withdrawal failure'); END;",
        )
        .unwrap();
    drop(ChildApprovalCleanupGuard {
        db: db.clone(),
        id: id.clone(),
        parent_agent_id: parent.clone(),
        subagent_id: "failed-child".into(),
        conversation_id: None,
        armed: true,
    });
    let later = HeadlessQueueAdapter {
        db: db.clone(),
        parent_agent_id: parent.clone(),
        subagent_id: "later-child".into(),
    };
    let failure = later
        .request_permission("write_file", &json!({"path":"src/later"}))
        .await
        .unwrap_err();
    assert!(failure.contains("injected withdrawal failure"));
    assert_eq!(
        cade_store::sqlite::list_pending_approvals(&db)
            .unwrap()
            .len(),
        1,
        "a failed cleanup must not publish another actionable request"
    );
    assert!(
        cade_store::sqlite::global_events_after(&db, 0)
            .unwrap()
            .is_empty(),
        "failed withdrawal cannot claim a resolution"
    );
    db.get()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_withdrawal;")
        .unwrap();
    assert!(cade_store::sqlite::resolve_pending_approval(&db, &id, "denied").unwrap());
    forget_child_approval_withdrawal(&parent, &id);
}

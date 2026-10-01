//! Working Session contracts through the real client, Run, and Subagent adapters.
use super::*;
use cade_core::permissions::{PermissionManager, PermissionMode, PermissionService};
use std::time::Duration;

async fn pending_id(state: &AppState, child: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(approval) = sqlite::list_pending_approvals(&state.db)
                .unwrap()
                .into_iter()
                .find(|approval| approval.subagent_id.as_deref() == Some(child))
            {
                return approval.id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child must publish its actionable approval")
}

async fn decide(
    state: &AppState,
    id: &str,
    action: &str,
) -> Result<axum::Json<Value>, (axum::http::StatusCode, String)> {
    crate::server::api::approvals::action_approval(
        State(state.clone()),
        Path(id.into()),
        Json(crate::server::api::approvals::ActionPayload {
            action: action.into(),
            feedback: None,
        }),
    )
    .await
}

fn queue(
    state: &AppState,
    permissions: &PermissionManager,
    child: &str,
) -> tokio::task::JoinHandle<Result<bool, String>> {
    let adapter = super::super::subagent::HeadlessQueueAdapter {
        db: state.db.clone(),
        parent_agent_id: "session-parent".into(),
        subagent_id: child.into(),
        permissions: permissions.clone(),
        permission_sessions: state.permission_sessions.clone(),
    };
    tokio::spawn(async move {
        adapter
            .request_permission(
                "write_file",
                &json!({"path":"file.txt", "content":"different call"}),
            )
            .await
    })
}

#[tokio::test]
async fn working_session_child_decision_releases_other_waiters_and_is_published_before_ack() {
    let state = build_state_with_llm(Arc::new(PanicOnCallLlm));
    approval_test_run(&state.db, "session-parent");
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let id = state.permission_sessions.open(&root).unwrap();
    let grants = state.permission_sessions.for_run(&id, &root).unwrap();
    let permissions =
        PermissionManager::new(PermissionMode::Default).with_session_grants(grants.clone());
    let first = queue(&state, &permissions, "first-child");
    let second = queue(&state, &permissions, "second-child");
    let first_id = pending_id(&state, "first-child").await;
    let second_id = pending_id(&state, "second-child").await;
    let _ = decide(&state, &first_id, "approve_session").await.unwrap();
    assert!(
        grants.allows("write_file"),
        "grant publication precedes the acknowledgement"
    );
    for waiter in [first, second] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), waiter)
                .await
                .unwrap()
                .unwrap(),
            Ok(true)
        );
    }
    assert_eq!(
        sqlite::get_approval_status(&state.db, &second_id)
            .unwrap()
            .as_deref(),
        Some("approved_session")
    );
    assert!(
        sqlite::list_pending_approvals(&state.db)
            .unwrap()
            .is_empty()
    );
    state.permission_sessions.close(&id);
    assert!(
        !permissions
            .resolve("write_file", &json!({"path":"other.txt"}), false)
            .is_allow()
    );
}

#[tokio::test]
async fn working_session_remote_grant_resolves_the_parent_dialog_in_live_stream_and_replay() {
    use cade_agent::tools::ApprovalDelegate;
    let state = build_state_with_llm(Arc::new(PanicOnCallLlm));
    let run_id = approval_test_run(&state.db, "session-parent");
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let session = state.permission_sessions.open(&root).unwrap();
    let grants = state.permission_sessions.for_run(&session, &root).unwrap();
    let permissions = PermissionManager::new(PermissionMode::Default).with_session_grants(grants);
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let delegate = super::super::execution::SseApprovalDelegate {
        db: state.db.clone(),
        agent_id: "session-parent".into(),
        run_id: run_id.clone(),
        conversation_id: None,
        permission_sessions: state.permission_sessions.clone(),
        permissions: permissions.clone(),
        tx,
    };
    let parent = tokio::spawn(async move {
        delegate
            .request_approval(
                "parent-call",
                "write_file",
                &json!({"path":"parent.txt", "content":"parent"}),
                "test",
                None,
            )
            .await
    });
    let request: Value = serde_json::from_str(&rx.recv().await.unwrap().unwrap().data).unwrap();
    assert_eq!(request["message_type"], "approval_required");
    let child = queue(&state, &permissions, "granting-child");
    let approval = pending_id(&state, "granting-child").await;
    let _ = decide(&state, &approval, "approve_session").await.unwrap();
    assert!(child.await.unwrap().unwrap());
    assert!(
        tokio::time::timeout(Duration::from_secs(3), parent)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    let resolved: Value = serde_json::from_str(&rx.recv().await.unwrap().unwrap().data).unwrap();
    assert_eq!(resolved["message_type"], "approval_resolved");
    assert_eq!(resolved["id"], request["id"]);
    assert_eq!(resolved["status"], "approved_session");
    assert!(resolved["seq_id"].as_i64().unwrap() > request["seq_id"].as_i64().unwrap());
    let events = sqlite::run_events_after(&state.db, &run_id, -1).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|(_, text)| text.contains("approval_resolved"))
            .count(),
        1
    );
    state.permission_sessions.close(&session);
    assert!(
        crate::server::permission_sessions::approval_outcome(
            "approved_session",
            "write_file",
            &permissions
        )
        .is_err(),
        "an inherited resolution must remain session-dependent"
    );
}

#[tokio::test]
async fn working_session_rejected_or_cancelled_decisions_do_not_grant_tools() {
    let state = build_state_with_llm(Arc::new(PanicOnCallLlm));
    approval_test_run(&state.db, "session-parent");
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let id = state.permission_sessions.open(&root).unwrap();
    let grants = state.permission_sessions.for_run(&id, &root).unwrap();
    let permissions =
        PermissionManager::new(PermissionMode::Default).with_session_grants(grants.clone());
    let worker = queue(&state, &permissions, "cancelled");
    let approval = pending_id(&state, "cancelled").await;
    sqlite::resolve_pending_approval(&state.db, &approval, "denied:cancelled").unwrap();
    assert!(decide(&state, &approval, "approve_session").await.is_err());
    assert!(!grants.allows("write_file"));
    assert!(worker.await.unwrap().is_err());
    let worker = queue(&state, &permissions, "closed");
    let approval = pending_id(&state, "closed").await;
    state.permission_sessions.close(&id);
    assert!(decide(&state, &approval, "approve_session").await.is_err());
    assert!(!grants.allows("write_file"));
    let _ = decide(&state, &approval, "deny").await.unwrap();
    assert_eq!(worker.await.unwrap(), Ok(false));
    let worker = queue(
        &state,
        &PermissionManager::new(PermissionMode::Default),
        "no-session",
    );
    let approval = pending_id(&state, "no-session").await;
    assert!(decide(&state, &approval, "approve_session").await.is_err());
    let _ = decide(&state, &approval, "approve").await.unwrap();
    assert_eq!(worker.await.unwrap(), Ok(true));
}

#[tokio::test]
async fn working_session_http_client_owns_scope_across_reconnects_and_closes_on_drop() {
    let mut state = build_state_with_llm(Arc::new(PanicOnCallLlm));
    Arc::make_mut(&mut state.config).api_key = Some("working-session-test-key".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::server::api::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = cade_agent::agent::client::HttpTransport::new(
        format!("http://{address}"),
        "working-session-test-key".into(),
    )
    .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let session = client.open_working_session(&root).await.unwrap();
    let options = session.execution_options().await.unwrap();
    let id = options["working_session_id"].as_str().unwrap();
    assert_eq!(options["cwd"], json!(root));
    let grants = state.permission_sessions.for_run(id, &root).unwrap();
    grants.grant_if("write_file", || Ok(true)).unwrap();
    // A fresh HTTP connection renews the same client-owned identity.
    let reconnected = cade_agent::agent::client::HttpTransport::new(
        format!("http://{address}"),
        "working-session-test-key".into(),
    )
    .unwrap();
    reconnected
        .raw_post(&format!("/working-sessions/{id}/heartbeat"), &json!({}))
        .await
        .unwrap();
    assert!(
        state
            .permission_sessions
            .for_run(id, &root)
            .unwrap()
            .allows("write_file")
    );
    let fresh = client.open_working_session(&root).await.unwrap();
    let fresh_options = fresh.execution_options().await.unwrap();
    let fresh_id = fresh_options["working_session_id"].as_str().unwrap();
    assert_ne!(id, fresh_id);
    assert!(
        !state
            .permission_sessions
            .for_run(fresh_id, &root)
            .unwrap()
            .allows("write_file")
    );
    session.close().await.unwrap();
    assert!(!grants.allows("write_file"));
    assert!(state.permission_sessions.for_run(id, &root).is_err());
    drop(fresh);
    tokio::time::timeout(Duration::from_secs(3), async {
        while state.permission_sessions.for_run(fresh_id, &root).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropping an interrupted client must close its Working Session");
    let recovering = client.open_working_session(&root).await.unwrap();
    let old_options = recovering.execution_options().await.unwrap();
    let old_id = old_options["working_session_id"].as_str().unwrap();
    state
        .permission_sessions
        .for_run(old_id, &root)
        .unwrap()
        .grant_if("write_file", || Ok(true))
        .unwrap();
    // A missing identity is what the client sees after lease expiry or restart.
    state.permission_sessions.close(old_id);
    let new_options = recovering.execution_options().await.unwrap();
    let new_id = new_options["working_session_id"].as_str().unwrap();
    assert_ne!(old_id, new_id);
    assert!(
        !state
            .permission_sessions
            .for_run(new_id, &root)
            .unwrap()
            .allows("write_file")
    );
    recovering.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn working_session_already_running_subagent_observes_late_grant_from_another_child() {
    struct ChildWriter {
        started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for ChildWriter {
        async fn complete(
            &self,
            request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let mut tools = vec![];
            if let Some(wait) = self.release.lock().await.take() {
                self.started
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(())
                    .unwrap();
                wait.await.unwrap();
                tools.push(LlmToolCall {
                    id: "child-write".into(),
                    name: "WriteFileGemini".into(),
                    arguments: json!({"path":"child.txt", "content":"late grant works"}),
                    thought_signature: None,
                });
            } else {
                assert!(
                    !format!("{:?}", request.messages).contains("Tool error:"),
                    "{:?}",
                    request.messages
                );
            }
            Ok(cade_ai::CompletionResponse {
                content: Some("done".into()),
                tool_calls: tools,
                finish_reason: "stop".into(),
            })
        }
        async fn stream(
            &self,
            _: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!()
        }
    }
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let state = build_state_with_llm(Arc::new(ChildWriter {
        started: std::sync::Mutex::new(Some(started)),
        release: tokio::sync::Mutex::new(Some(wait)),
    }));
    approval_test_run(&state.db, "session-parent");
    // The child inherits the advertised alias, while its sibling grants the
    // canonical identity. Permission reuse must not bypass tool inheritance.
    seed_test_tools(&state.db, "session-parent", &["WriteFileGemini"]);
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let id = state.permission_sessions.open(&root).unwrap();
    let execution = runtime::RunExecutionOptions {
        cwd: Some(root.clone()),
        working_session_id: Some(id.clone()),
        permissions: Some(Default::default()),
        allowed_paths: Some(vec![".".into()]),
        ..Default::default()
    };
    let child_state = state.clone();
    let child = tokio::spawn(async move {
        super::super::launch_subagent_handler(
            State(child_state),
            Path("session-parent".into()),
            Json(super::super::LaunchSubagentPayload {
                conversation_id: None,
                args: json!({"prompt":"write child.txt", "enforce_isolation":false}),
                mode: "default".into(),
                execution,
            }),
        )
        .await
        .unwrap()
        .0
    });
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();
    let grants = state.permission_sessions.for_run(&id, &root).unwrap();
    let permissions = PermissionManager::new(PermissionMode::Default).with_session_grants(grants);
    let other_child = queue(&state, &permissions, "granting-child");
    let approval = pending_id(&state, "granting-child").await;
    let _ = decide(&state, &approval, "approve_session").await.unwrap();
    assert_eq!(other_child.await.unwrap(), Ok(true));
    release.send(()).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), child)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome["is_error"], false, "{outcome}");
    assert_eq!(
        std::fs::read_to_string(root.join("child.txt")).unwrap(),
        "late grant works"
    );
    assert!(
        sqlite::list_pending_approvals(&state.db)
            .unwrap()
            .is_empty()
    );
}

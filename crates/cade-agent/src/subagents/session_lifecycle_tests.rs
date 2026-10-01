//! Behavioral lifecycle and real native-runtime regressions. No process-wide
//! cwd mutation: concurrent tests/children must use their own execution paths.

use super::*;
use cade_core::permissions::PermissionMode;
use std::path::PathBuf;

fn config() -> SubagentConfig {
    SubagentConfig::from_args(&json!({"prompt":"task", "enforce_isolation":true}))
}

fn done() -> SubagentOutcome {
    SubagentOutcome::Done {
        summary: "child completed".into(),
        iterations: 0,
        tool_calls_count: 0,
        token_usage: 0,
    }
}

fn policy() -> SubagentToolPolicy {
    SubagentToolPolicy {
        permissions: PermissionManager::new(PermissionMode::BypassPermissions),
        tools: SubagentTools::All,
        inherited_tools: ["write_file", "read_file", "edit_file", "bash"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        allow_nesting: false,
        max_depth: 2,
    }
}

struct ScriptedLlm(Mutex<Vec<SubagentTurnResponse>>);

impl ScriptedLlm {
    fn tools(calls: Vec<(&str, Value)>) -> Self {
        Self(Mutex::new(vec![
            SubagentTurnResponse {
                content: None,
                tool_calls: calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, arguments))| SubagentToolCall {
                        id: format!("tool-{i}"),
                        name: name.into(),
                        arguments,
                    })
                    .collect(),
                tokens_used: 3,
            },
            SubagentTurnResponse {
                content: None,
                tool_calls: vec![SubagentToolCall {
                    id: "finish".into(),
                    name: "finish".into(),
                    arguments: json!({"status":"done", "summary":"child completed"}),
                }],
                tokens_used: 5,
            },
        ]))
    }
}

#[async_trait]
impl SubagentLlmExecutor for ScriptedLlm {
    async fn complete_turn(
        &self,
        _: &str,
        _: &str,
        _: &[SubagentMessage],
        _: &[Value],
    ) -> Result<SubagentTurnResponse, String> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

struct Cleanup {
    calls: Arc<Mutex<Vec<bool>>>,
    fail: bool,
}

#[async_trait]
impl SubagentCleanup for Cleanup {
    async fn finalize(&mut self, success: bool) -> Result<(), String> {
        self.calls.lock().unwrap().push(success);
        if self.fail {
            Err("injected ephemeral deletion failure".into())
        } else {
            Ok(())
        }
    }

    fn discard(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().push(false);
        if self.fail {
            Err("injected ephemeral deletion failure".into())
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn session_finalization_caches_actual_merge_failure_and_emits_once_after_cleanup() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("conflict"), "baseline").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let (tx, mut events) = tokio::sync::mpsc::channel(8);
    let mut session = SubagentSession::new(config(), "parent")
        .with_event_emitter(SubagentEventEmitter::new(Some(tx)))
        .with_cleanup(Box::new(Cleanup {
            calls: calls.clone(),
            fail: false,
        }));
    let control = session.register_control(true);
    let receipt = session.completion();
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    std::fs::write(child.join("conflict"), "child").unwrap();
    std::fs::write(child.join("new"), "must not apply").unwrap();
    std::fs::write(host.path().join("conflict"), "parent").unwrap();
    let outcome = session.finalize_outcome(done()).await;
    assert!(!outcome.is_success());
    assert!(outcome.summary_text().contains("child completed"));
    assert!(outcome.summary_text().contains("conflict"));
    assert_eq!(receipt.outcome(), Some(outcome.clone()));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![false],
        "merge failure must discard findings"
    );
    assert!(!child.exists());
    assert!(!host.path().join("new").exists());
    assert_eq!(
        std::fs::read_to_string(host.path().join("conflict")).unwrap(),
        "parent"
    );
    assert_eq!(session.finalize_outcome(done()).await, outcome);
    assert!(
        matches!(events.recv().await, Some(SubagentEvent::Finished { outcome: published }) if published == outcome)
    );
    assert!(events.try_recv().is_err());
    assert!(control.set_paused(false).is_err());
    assert!(control.request_model("late".into()).is_err());
    assert!(control.steer("late".into()).is_err());
    drop(session);
    assert_eq!(*calls.lock().unwrap(), vec![false]);
}

#[tokio::test]
async fn session_all_terminal_outcomes_cleanup_once_and_only_done_merges() {
    let outcomes = [
        done(),
        SubagentOutcome::Blocked {
            reason: "blocked".into(),
            questions: vec!["question".into()],
        },
        SubagentOutcome::Failed {
            error: "failed".into(),
        },
        SubagentOutcome::Exhausted {
            reason: "budget".into(),
            iterations: 2,
            tokens_used: 8,
        },
    ];
    for outcome in outcomes {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("file"), "baseline").unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let mut session =
            SubagentSession::new(config(), "parent").with_cleanup(Box::new(Cleanup {
                calls: calls.clone(),
                fail: false,
            }));
        session.prepare_workspace(host.path(), None).await.unwrap();
        let child = session.execution_path(host.path()).to_path_buf();
        std::fs::write(child.join("file"), "child").unwrap();
        let finalized = session.finalize_outcome(outcome.clone()).await;
        assert_eq!(finalized, outcome);
        assert_eq!(*calls.lock().unwrap(), vec![outcome.is_success()]);
        assert_eq!(
            std::fs::read_to_string(host.path().join("file")).unwrap(),
            if outcome.is_success() {
                "child"
            } else {
                "baseline"
            }
        );
        assert!(!child.exists());
        drop(session);
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn session_cleanup_failure_is_part_of_actual_terminal_outcome() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("file"), "baseline").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let (tx, mut events) = tokio::sync::mpsc::channel(4);
    let mut session = SubagentSession::new(config(), "parent")
        .with_event_emitter(SubagentEventEmitter::new(Some(tx)))
        .with_cleanup(Box::new(Cleanup {
            calls: calls.clone(),
            fail: true,
        }));
    let receipt = session.completion();
    session.prepare_workspace(host.path(), None).await.unwrap();
    std::fs::write(
        session.execution_path(host.path()).join("file"),
        "merged before cleanup failed",
    )
    .unwrap();
    let result = session.finalize_outcome(done()).await;
    assert!(matches!(result, SubagentOutcome::Failed { .. }));
    assert!(
        result
            .summary_text()
            .contains("injected ephemeral deletion failure")
    );
    assert!(
        result
            .summary_text()
            .contains("isolated workspace reconciliation succeeded")
    );
    assert_eq!(
        std::fs::read_to_string(host.path().join("file")).unwrap(),
        "merged before cleanup failed"
    );
    assert_eq!(receipt.outcome(), Some(result.clone()));
    assert!(
        matches!(events.recv().await, Some(SubagentEvent::Finished { outcome }) if outcome == result)
    );
    assert_eq!(session.finalize_outcome(done()).await, result);
    drop(session);
    assert_eq!(*calls.lock().unwrap(), vec![true]);
}

#[tokio::test]
async fn session_cleanup_stack_finalizes_later_resources_first_and_keeps_failures() {
    let earlier = Arc::new(Mutex::new(vec![]));
    let later = Arc::new(Mutex::new(vec![]));
    let mut session = SubagentSession::new(config(), "parent")
        .with_cleanup(Box::new(Cleanup {
            calls: earlier.clone(),
            fail: false,
        }))
        .with_cleanup(Box::new(Cleanup {
            calls: later.clone(),
            fail: true,
        }));
    let result = session.finalize_outcome(done()).await;
    assert!(!result.is_success());
    assert_eq!(*later.lock().unwrap(), vec![true]);
    assert_eq!(
        *earlier.lock().unwrap(),
        vec![false],
        "remaining cleanup receives actual failed status"
    );
    drop(session);
    assert_eq!(earlier.lock().unwrap().len(), 1);
    assert_eq!(later.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn session_terminal_event_survives_a_full_telemetry_channel_and_owner_drop() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(SubagentEvent::Thought {
        text: "older telemetry".into(),
    })
    .await
    .unwrap();
    let mut session = SubagentSession::new(config(), "parent")
        .with_event_emitter(SubagentEventEmitter::new(Some(tx)));
    let result = tokio::time::timeout(Duration::from_secs(1), session.finalize_outcome(done()))
        .await
        .unwrap();
    assert_eq!(session.completion().outcome(), Some(result.clone()));
    drop(session);
    assert!(matches!(
        rx.recv().await,
        Some(SubagentEvent::Thought { .. })
    ));
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, SubagentEvent::Finished { outcome } if outcome == result));
    assert!(rx.try_recv().is_err());
}

struct BlockedWriter {
    entered: tokio::sync::mpsc::Sender<PathBuf>,
    panic: bool,
}

#[async_trait]
impl SubagentToolExecutor for BlockedWriter {
    async fn execute_tool(
        &self,
        _: &str,
        _: &str,
        _: &Value,
        cwd: &Path,
    ) -> Result<String, String> {
        std::fs::write(cwd.join("file"), "interrupted child").unwrap();
        self.entered.send(cwd.to_path_buf()).await.unwrap();
        assert!(!self.panic, "injected native execution panic");
        std::future::pending().await
    }
}

#[tokio::test]
async fn session_controlled_cancel_timeout_and_panic_finalize_before_returning() {
    for reason in [
        SubagentLaunchFailure::Cancelled,
        SubagentLaunchFailure::TimedOut,
        SubagentLaunchFailure::Panicked,
    ] {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("file"), "baseline").unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let (event_tx, mut events) = tokio::sync::mpsc::channel(32);
        let mut session = SubagentSession::new(config(), "parent")
            .with_tool_policy(policy())
            .with_event_emitter(SubagentEventEmitter::new(Some(event_tx)))
            .with_cleanup(Box::new(Cleanup {
                calls: calls.clone(),
                fail: false,
            }));
        let control = session.register_control(true);
        let receipt = session.completion();
        session.prepare_workspace(host.path(), None).await.unwrap();
        let child = session.execution_path(host.path()).to_path_buf();
        let (entered_tx, mut entered) = tokio::sync::mpsc::channel(1);
        let tool = BlockedWriter {
            entered: entered_tx,
            panic: reason == SubagentLaunchFailure::Panicked,
        };
        let llm = ScriptedLlm::tools(vec![(
            "write_file",
            json!({"path":"file", "content":"child"}),
        )]);
        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel(1);
        let cancellation = SubagentCancellation::new(cancel_tx).with_completion(receipt.clone());
        let mut run = Box::pin(session.run_controlled(
            &llm,
            &tool,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            host.path(),
            Duration::from_millis(100),
            &mut cancel_rx,
        ));
        if reason != SubagentLaunchFailure::Panicked {
            tokio::select! {
                path = entered.recv() => assert_eq!(path.unwrap(), child),
                _ = &mut run => panic!("child finished before blocked execution"),
            }
            if reason == SubagentLaunchFailure::Cancelled {
                cancellation.cancel().unwrap();
            }
        }
        let outcome = run.await;
        assert!(!outcome.is_success());
        assert_eq!(receipt.failure(), Some(reason));
        assert_eq!(receipt.outcome(), Some(outcome.clone()));
        assert!(!child.exists());
        assert_eq!(
            std::fs::read_to_string(host.path().join("file")).unwrap(),
            "baseline"
        );
        assert_eq!(*calls.lock().unwrap(), vec![false]);
        let expected = match reason {
            SubagentLaunchFailure::Cancelled => "cancelled",
            SubagentLaunchFailure::TimedOut => "timeout",
            _ => "error",
        };
        assert_eq!(
            control.status(),
            SubagentStatus::Finished {
                outcome: expected.into()
            }
        );
        assert!(cancellation.cancel().is_err());
        assert_eq!(session.finalize_outcome(done()).await, outcome);
        let mut finished = 0;
        while let Ok(event) = events.try_recv() {
            if let SubagentEvent::Finished { outcome: published } = event {
                assert_eq!(published, outcome);
                finished += 1;
            }
        }
        assert_eq!(finished, 1);
    }
}

#[tokio::test]
async fn session_controlled_timeout_waiting_for_merge_lock_discards_without_host_writes() {
    let host = tempfile::tempdir().unwrap();
    let file = host.path().join("file");
    std::fs::write(&file, "baseline").unwrap();
    let mut session = SubagentSession::new(config(), "parent");
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    std::fs::write(child.join("file"), "child").unwrap();
    let receipt = session.completion();
    let held = crate::tools::file_lock::FileLockManager::global()
        .acquire_lock(&file)
        .await;
    let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
    let llm = ScriptedLlm::tools(vec![]);
    let tools = BlockedWriter {
        entered: tokio::sync::mpsc::channel(1).0,
        panic: false,
    };
    let result = session
        .run_controlled(
            &llm,
            &tools,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            host.path(),
            Duration::from_millis(20),
            &mut rx,
        )
        .await;
    assert!(
        result
            .summary_text()
            .contains("timeout during finalization")
    );
    assert_eq!(receipt.failure(), Some(SubagentLaunchFailure::TimedOut));
    assert_eq!(receipt.outcome(), Some(result.clone()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "baseline");
    assert!(!child.exists());
    drop(held);
    assert_eq!(session.finalize_outcome(done()).await, result);
    assert_eq!(std::fs::read_to_string(file).unwrap(), "baseline");
}

#[tokio::test]
async fn session_controlled_cleanup_timeout_reports_reconciled_workspace_and_discards_resources() {
    struct PendingCleanup(Arc<Mutex<Vec<bool>>>);
    #[async_trait]
    impl SubagentCleanup for PendingCleanup {
        async fn finalize(&mut self, success: bool) -> Result<(), String> {
            self.0.lock().unwrap().push(success);
            std::future::pending().await
        }
        fn discard(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().push(false);
            Ok(())
        }
    }
    let host = tempfile::tempdir().unwrap();
    let file = host.path().join("file");
    std::fs::write(&file, "baseline").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut session = SubagentSession::new(config(), "parent")
        .with_cleanup(Box::new(PendingCleanup(calls.clone())));
    session.prepare_workspace(host.path(), None).await.unwrap();
    std::fs::write(
        session.execution_path(host.path()).join("file"),
        "reconciled child",
    )
    .unwrap();
    let receipt = session.completion();
    let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
    let llm = ScriptedLlm::tools(vec![]);
    let tools = BlockedWriter {
        entered: tokio::sync::mpsc::channel(1).0,
        panic: false,
    };
    let result = session
        .run_controlled(
            &llm,
            &tools,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            host.path(),
            Duration::from_millis(20),
            &mut rx,
        )
        .await;
    assert!(!result.is_success());
    assert!(
        result
            .summary_text()
            .contains("timeout during finalization")
    );
    assert!(
        result
            .summary_text()
            .contains("isolated workspace reconciliation succeeded")
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "reconciled child");
    assert_eq!(*calls.lock().unwrap(), vec![true, false]);
    assert_eq!(receipt.outcome(), Some(result));
    drop(session);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![true, false],
        "discard runs once after interrupted cleanup"
    );
}

#[tokio::test]
async fn session_controlled_finalizer_panic_and_discard_failure_enter_actual_outcome() {
    struct PanickingCleanup;
    #[async_trait]
    impl SubagentCleanup for PanickingCleanup {
        async fn finalize(&mut self, _: bool) -> Result<(), String> {
            panic!("finalizer panicked")
        }
        fn discard(&mut self) -> Result<(), String> {
            Err("discard also failed".into())
        }
    }
    let host = tempfile::tempdir().unwrap();
    let mut session =
        SubagentSession::new(config(), "parent").with_cleanup(Box::new(PanickingCleanup));
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    let receipt = session.completion();
    let llm = ScriptedLlm::tools(vec![]);
    let tools = BlockedWriter {
        entered: tokio::sync::mpsc::channel(1).0,
        panic: false,
    };
    let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
    let result = session
        .run_controlled(
            &llm,
            &tools,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            host.path(),
            Duration::from_secs(1),
            &mut rx,
        )
        .await;
    assert!(
        result
            .summary_text()
            .contains("panicked during finalization")
    );
    assert!(result.summary_text().contains("discard also failed"));
    assert_eq!(receipt.failure(), Some(SubagentLaunchFailure::Panicked));
    assert_eq!(receipt.outcome(), Some(result));
    assert!(!child.exists());
}

#[tokio::test]
async fn session_queued_admission_failures_cleanup_and_publish_actual_status() {
    for reason in [
        SubagentLaunchFailure::Cancelled,
        SubagentLaunchFailure::TimedOut,
        SubagentLaunchFailure::Closed,
    ] {
        let host = tempfile::tempdir().unwrap();
        let mut session = SubagentSession::new(config(), "parent");
        let control = session.register_control(true);
        let receipt = session.completion();
        session.prepare_workspace(host.path(), None).await.unwrap();
        let child = session.execution_path(host.path()).to_path_buf();
        std::fs::write(child.join("new"), "queued child must not merge").unwrap();
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().acquire_owned().await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        if reason == SubagentLaunchFailure::Cancelled {
            tx.send(()).await.unwrap();
        }
        if reason == SubagentLaunchFailure::Closed {
            slots.close();
        }
        assert_eq!(
            session
                .acquire_slot(slots.clone(), Duration::from_millis(10), &mut rx)
                .await
                .unwrap_err(),
            reason
        );
        assert_eq!(receipt.failure(), Some(reason));
        assert!(!receipt.outcome().unwrap().is_success());
        assert!(!child.exists());
        assert!(!host.path().join("new").exists());
        assert!(matches!(control.status(), SubagentStatus::Finished { .. }));
        assert_eq!(slots.available_permits(), 0);
        drop(held);
    }
}

#[tokio::test]
async fn session_drop_receipt_contains_workspace_and_resource_cleanup_failures() {
    let host = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut session = SubagentSession::new(config(), "parent").with_cleanup(Box::new(Cleanup {
        calls: calls.clone(),
        fail: true,
    }));
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    std::fs::remove_dir(&child).unwrap();
    std::fs::write(&child, "invalid temporary directory").unwrap();
    let receipt = session.completion();
    drop(session);
    let result = receipt.outcome().unwrap();
    assert!(
        result
            .summary_text()
            .contains("workspace finalization failed")
    );
    assert!(
        result
            .summary_text()
            .contains("injected ephemeral deletion failure")
    );
    assert_eq!(*calls.lock().unwrap(), vec![false]);
    std::fs::remove_file(child).unwrap();
}

#[tokio::test]
async fn session_background_cancellation_delivers_actual_receipt_after_cleanup() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("file"), "baseline").unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut session = SubagentSession::new(config(), "parent")
        .with_tool_policy(policy())
        .with_cleanup(Box::new(Cleanup {
            calls: calls.clone(),
            fail: true,
        }));
    session.register_control(true);
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    let receipt = session.completion();
    let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel(1);
    let cancellation = SubagentCancellation::new(cancel_tx).with_completion(receipt.clone());
    let (entered_tx, mut entered) = tokio::sync::mpsc::channel(1);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let primary = host.path().to_path_buf();
    session.launch_background(
        Arc::new(Semaphore::new(1)),
        Duration::from_secs(2),
        cancel_rx,
        move |mut session, permit| async move {
            let _permit = permit;
            session
                .run_autonomous_loop(
                    &ScriptedLlm::tools(vec![("write_file", json!({"path":"file"}))]),
                    &BlockedWriter {
                        entered: entered_tx,
                        panic: false,
                    },
                    "test".into(),
                    "system".into(),
                    "task".into(),
                    vec![],
                    vec![],
                    &primary,
                )
                .await
        },
        {
            let receipt = receipt.clone();
            let child = child.clone();
            let calls = calls.clone();
            move |result| async move {
                assert_eq!(result.unwrap_err(), SubagentLaunchFailure::Cancelled);
                assert!(!child.exists(), "delivery follows workspace teardown");
                assert_eq!(*calls.lock().unwrap(), vec![false]);
                let outcome = receipt
                    .outcome()
                    .expect("terminal cleanup result available at delivery");
                assert!(
                    outcome
                        .summary_text()
                        .contains("injected ephemeral deletion failure")
                );
                done_tx.send(outcome).unwrap();
            }
        },
    );
    tokio::time::timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    cancellation.cancel().unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(2), done_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.outcome(), Some(outcome));
    assert_eq!(
        std::fs::read_to_string(host.path().join("file")).unwrap(),
        "baseline"
    );
    assert!(cancellation.cancel().is_err());
}

#[tokio::test]
async fn session_background_constructor_panic_still_cleans_and_delivers_once() {
    let calls = Arc::new(Mutex::new(vec![]));
    let session = SubagentSession::new(config(), "parent").with_cleanup(Box::new(Cleanup {
        calls: calls.clone(),
        fail: false,
    }));
    let receipt = session.completion();
    let (tx, rx) = tokio::sync::oneshot::channel();
    session.launch_background(
        Arc::new(Semaphore::new(1)),
        Duration::from_secs(2),
        tokio::sync::mpsc::channel(1).1,
        |_, _| -> std::future::Ready<()> { panic!("child constructor panicked") },
        move |result| async move {
            tx.send(result).unwrap();
        },
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        SubagentLaunchFailure::Panicked
    );
    assert_eq!(receipt.failure(), Some(SubagentLaunchFailure::Panicked));
    assert!(
        receipt
            .outcome()
            .unwrap()
            .summary_text()
            .contains("panicked")
    );
    assert_eq!(*calls.lock().unwrap(), vec![false]);
}

#[tokio::test]
async fn session_finalization_commit_boundary_refuses_late_controls_and_cancel() {
    struct GatedCleanup {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl SubagentCleanup for GatedCleanup {
        async fn finalize(&mut self, success: bool) -> Result<(), String> {
            assert!(success);
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        }
        fn discard(&mut self) -> Result<(), String> {
            panic!("committed cleanup must not be interrupted")
        }
    }
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut session =
        SubagentSession::new(config(), "parent").with_cleanup(Box::new(GatedCleanup {
            entered: entered.clone(),
            release: release.clone(),
        }));
    let control = session.register_control(true);
    let receipt = session.completion();
    let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel(1);
    let cancellation = SubagentCancellation::new(cancel_tx).with_completion(receipt.clone());
    let (tx, rx) = tokio::sync::oneshot::channel();
    session.launch_background(
        Arc::new(Semaphore::new(1)),
        Duration::from_secs(2),
        cancel_rx,
        |mut session, permit| async move {
            let _permit = permit;
            session.finalize_outcome(done()).await
        },
        move |result| async move {
            tx.send(result).unwrap();
        },
    );
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    assert!(receipt.is_finishing());
    assert!(
        receipt.outcome().is_none(),
        "receipt waits for actual cleanup completion"
    );
    assert!(cancellation.cancel().is_err());
    assert!(control.set_paused(false).is_err());
    assert!(control.steer("late".into()).is_err());
    assert!(control.request_model("late".into()).is_err());
    release.notify_one();
    let outcome = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(outcome.is_success());
    assert_eq!(receipt.outcome(), Some(outcome));
}

#[tokio::test]
async fn session_acknowledged_cancel_between_last_turn_and_finalization_cannot_merge() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("file"), "baseline").unwrap();
    let mut session = SubagentSession::new(config(), "parent");
    session.prepare_workspace(host.path(), None).await.unwrap();
    std::fs::write(session.execution_path(host.path()).join("file"), "child").unwrap();
    let receipt = session.completion();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let cancellation = SubagentCancellation::new(tx).with_completion(receipt.clone());
    cancellation.cancel().unwrap();
    let result = session.finalize_outcome(done()).await;
    assert!(!result.is_success());
    assert_eq!(receipt.failure(), Some(SubagentLaunchFailure::Cancelled));
    assert_eq!(
        std::fs::read_to_string(host.path().join("file")).unwrap(),
        "baseline"
    );
    assert!(cancellation.cancel().is_err());
}

struct NativeRuntimeTools {
    child: PathBuf,
    host: PathBuf,
    file: String,
    shell_file: String,
}

#[async_trait]
impl SubagentToolExecutor for NativeRuntimeTools {
    async fn execute_tool(
        &self,
        id: &str,
        name: &str,
        args: &Value,
        cwd: &Path,
    ) -> Result<String, String> {
        assert_eq!(cwd, self.child.as_path());
        let storage = Arc::new(
            crate::agent::HttpTransport::new("http://localhost:0".into(), "unused".into()).unwrap(),
        );
        let runtime = crate::tools::runtime::ToolRuntime::new(
            storage,
            Arc::new(crate::mcp::McpManager::empty()),
            "child".into(),
            cwd.to_path_buf(),
        );
        let result = runtime.execute(id.into(), name, args).await.unwrap();
        assert!(
            !result.is_error,
            "actual {name} execution failed: {}",
            result.output
        );
        if name == "write_file" {
            assert_eq!(
                std::fs::read_to_string(self.child.join(&self.file)).unwrap(),
                "child native write"
            );
        }
        if name == "edit_file" {
            assert_eq!(
                std::fs::read_to_string(self.child.join(&self.file)).unwrap(),
                "child native write edited"
            );
        }
        if name == "read_file" {
            assert!(result.output.contains("child native write"));
        }
        if name == "bash"
            && args["command"].as_str() == Some(if cfg!(windows) { "cd" } else { "pwd" })
        {
            assert!(
                result.output.contains(&self.child.display().to_string()),
                "shell ran outside child's actual cwd: {}",
                result.output
            );
        }
        assert!(
            !self.host.join(&self.file).exists(),
            "file tool escaped isolation"
        );
        assert!(
            !self.host.join(&self.shell_file).exists(),
            "shell tool escaped isolation"
        );
        Ok(result.output)
    }
}

#[tokio::test]
async fn session_native_file_and_shell_execution_use_actual_child_cwd_then_merge() {
    let host = tempfile::tempdir().unwrap();
    let mut session = SubagentSession::new(config(), "parent").with_tool_policy(policy());
    session.prepare_workspace(host.path(), None).await.unwrap();
    let child = session.execution_path(host.path()).to_path_buf();
    let file = format!("native-{}.txt", uuid::Uuid::new_v4());
    let shell_file = format!("shell-{}.txt", uuid::Uuid::new_v4());
    let sentinel = format!("sentinel-{}.txt", uuid::Uuid::new_v4());
    std::fs::write(child.join(&sentinel), "child native write").unwrap();
    let tools = NativeRuntimeTools {
        child: child.clone(),
        host: host.path().to_path_buf(),
        file: file.clone(),
        shell_file: shell_file.clone(),
    };
    let shell_command = if cfg!(windows) {
        format!("(echo child shell write) > {shell_file}")
    } else {
        format!("printf 'child shell write' > '{shell_file}'")
    };
    let llm = ScriptedLlm::tools(vec![
        // Probe shell cwd and a child-only read before mutating native calls.
        (
            "bash",
            json!({"command":if cfg!(windows) { "cd" } else { "pwd" }}),
        ),
        ("read_file", json!({"path":sentinel})),
        (
            "write_file",
            json!({"path":file, "content":"child native write"}),
        ),
        (
            "edit_file",
            json!({"path":file, "old_string":"child native write", "new_string":"child native write edited"}),
        ),
        ("read_file", json!({"path":file})),
        ("bash", json!({"command":shell_command})),
    ]);
    let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
    let outcome = session
        .run_controlled(
            &llm,
            &tools,
            "test".into(),
            "system".into(),
            "task".into(),
            vec![],
            vec![],
            host.path(),
            Duration::from_secs(10),
            &mut rx,
        )
        .await;
    assert!(outcome.is_success(), "{}", outcome.summary_text());
    assert_eq!(
        std::fs::read_to_string(host.path().join(file)).unwrap(),
        "child native write edited"
    );
    assert_eq!(
        std::fs::read_to_string(host.path().join(shell_file))
            .unwrap()
            .trim(),
        "child shell write"
    );
    assert!(!child.exists());
    assert!(matches!(
        outcome,
        SubagentOutcome::Done {
            iterations: 2,
            tool_calls_count: 7,
            token_usage: 8,
            ..
        }
    ));
}

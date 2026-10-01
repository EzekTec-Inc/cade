use cade_agent::agent::client::HttpTransport;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const START: &str =
    "data: {\"message_type\":\"stream_start\",\"run_id\":\"r-test\",\"seq_id\":0}\n\n";
const TEXT: &str = "data: {\"message_type\":\"assistant_message\",\"content\":\"hello\",\"run_id\":\"r-test\",\"seq_id\":1}\n\n";

fn done(status: &str) -> String {
    format!(
        "data: {{\"message_type\":\"run_done\",\"status\":\"{status}\",\"run_id\":\"r-test\",\"seq_id\":2}}\n\n"
    )
}

fn finalization_failure(persisted: bool, sequence: bool) -> String {
    let mut event = serde_json::json!({
        "message_type":"error", "run_id":"r-test", "error":"could not persist failed Run status: disk full",
        "code":"run_finalization_failed", "terminal_status_persisted":persisted,
    });
    if sequence {
        event["seq_id"] = 1.into();
    }
    format!("data: {event}\n\n")
}

async fn assert_gapped_fatal_retains_resume_cursor(first_is_replay: bool) {
    use cade_agent::agent::client::CadeMessage;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let fatal = serde_json::json!({"message_type":"error", "run_id":"r-gap", "seq_id":10,
            "code":"run_finalization_failed", "terminal_status_persisted":false, "error":"disk full"});
        let start =
            serde_json::json!({"message_type":"stream_start", "run_id":"r-gap", "seq_id":0});
        let mut journal = vec![start.clone()];
        journal.extend((1..10).map(|seq| serde_json::json!({"message_type":"assistant_message", "run_id":"r-gap", "seq_id":seq,"content":format!("{seq} ")})));
        journal.push(fatal.clone());
        for request_index in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            let events = if request_index == 0 {
                if first_is_replay {
                    assert!(path.ends_with("starting_after=-1"));
                } else {
                    assert_eq!(path, "/v1/agents/a-test/run");
                }
                vec![start.clone(), fatal.clone()]
            } else {
                assert!(path.starts_with("/v1/runs/r-gap/stream?starting_after="));
                let after: i64 = path
                    .split_once("starting_after=")
                    .unwrap()
                    .1
                    .parse()
                    .unwrap();
                assert_eq!(after, if request_index == 1 { 0 } else { 9 });
                journal
                    .iter()
                    .filter(|event| event["seq_id"].as_i64().unwrap() > after)
                    .cloned()
                    .collect()
            };
            let body = events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let client = HttpTransport::new(format!("http://{address}"), String::new()).unwrap();
    let cursor = std::cell::Cell::new(-1);
    let seen = std::cell::RefCell::new(Vec::<CadeMessage>::new());
    let record = |event: &CadeMessage| {
        if let Some(seq) = event.seq_id() {
            cursor.set(seq);
        }
        seen.borrow_mut().push(event.clone());
    };
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        if first_is_replay {
            client.resume_run("r-gap", -1, &record).await
        } else {
            client.start_run("a-test", "hello", None, &record).await
        }
    })
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("incomplete"));
    assert_eq!(
        cursor.get(),
        0,
        "out-of-order fatal diagnostic must not skip missing journal output"
    );
    assert_eq!(seen.borrow().last().unwrap().seq_id(), None);
    for expected_text in ["1 2 3 4 5 6 7 8 9 ", ""] {
        seen.borrow_mut().clear();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            client.resume_run("r-gap", cursor.get(), &record),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().to_string().contains("disk full"));
        assert_eq!(
            seen.borrow()
                .iter()
                .filter_map(|event| event.assistant_text())
                .collect::<String>(),
            expected_text
        );
        assert_eq!(
            cursor.get(),
            9,
            "unresolved finalization must remain replayable even when adjacent"
        );
        assert_eq!(
            seen.borrow()
                .iter()
                .filter(|event| event.msg_type() == "error")
                .count(),
            1
        );
        assert!(
            !seen
                .borrow()
                .iter()
                .any(|event| event.msg_type() == "run_done")
        );
    }
    server.await.unwrap();
}

#[tokio::test]
async fn live_gapped_fatal_diagnostic_preserves_cursor_and_recovers_missing_output() {
    assert_gapped_fatal_retains_resume_cursor(false).await;
}

#[tokio::test]
async fn replay_gapped_fatal_diagnostic_preserves_cursor_and_recovers_missing_output() {
    assert_gapped_fatal_retains_resume_cursor(true).await;
}

#[tokio::test]
async fn live_finalization_storage_failure_ends_as_incomplete_without_recovery() {
    for sequence in [true, false] {
        let diagnostic = finalization_failure(false, sequence);
        let (client, posts, _, server) = peer(
            format!("{START}{diagnostic}"),
            String::new(),
            "running",
            false,
            false,
        )
        .await;
        let seen = std::sync::Mutex::new(Vec::new());
        let result = tokio::time::timeout(
            Duration::from_millis(800),
            client.start_run("a-test", "hello", None, |event| {
                seen.lock().unwrap().push(event.clone())
            }),
        )
        .await;
        server.abort();
        let error = result
            .expect("explicit failed finalization must not retry a stale running status")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("incomplete") && error.contains("r-test") && error.contains("disk full"),
            "{error}"
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|m| m.msg_type() == "error")
                .count(),
            1
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.msg_type() == "run_done")
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn replay_finalization_storage_failure_ends_as_incomplete_without_terminal_outcome() {
    let diagnostic = finalization_failure(false, true);
    let (client, _, _, server) = peer(
        String::new(),
        format!("{START}{diagnostic}data: [DONE]\n\n"),
        "running",
        false,
        false,
    )
    .await;
    let result = tokio::time::timeout(
        Duration::from_millis(800),
        client.resume_run("r-test", -1, |_| {}),
    )
    .await;
    server.abort();
    let error = result
        .expect("replayed finalization failure must stop observation")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("incomplete") && error.contains("disk full"),
        "{error}"
    );
}

#[tokio::test]
async fn finalization_diagnostic_with_persisted_status_still_recovers_that_outcome() {
    let trace = format!("{START}{}", finalization_failure(true, true));
    let (client, _, _, server) = peer(trace.clone(), trace, "error", false, false).await;
    let messages = tokio::time::timeout(
        Duration::from_secs(1),
        client.start_run("a-test", "hello", None, |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(messages.last().unwrap().data["status"], "error");
    server.abort();
}

// Only the wire peer is fake: tests call the production HTTP observation interface.
async fn peer(
    live: String,
    replay: String,
    status: &str,
    hold: bool,
    hang_cancel: bool,
) -> (
    HttpTransport,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let posts = Arc::new(AtomicUsize::new(0));
    let cancels = Arc::new(AtomicUsize::new(0));
    let p = posts.clone();
    let c = cancels.clone();
    let status = status.to_owned();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (live, replay, status, p, c) = (
                live.clone(),
                replay.clone(),
                status.clone(),
                p.clone(),
                c.clone(),
            );
            connections.spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 { return; }
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap();
                let (body, sse, keep_open) = if path == "/v1/agents/a-test/run" {
                    p.fetch_add(1, Ordering::SeqCst);
                    (live, true, hold)
                } else if path.starts_with("/v1/runs/r-test/stream") {
                    (replay, true, hold)
                } else if path.ends_with("/cancel") {
                    c.fetch_add(1, Ordering::SeqCst);
                    if hang_cancel { std::future::pending::<()>().await; }
                    ("{\"status\":\"cancelling\"}".into(), false, false)
                } else if path == "/v1/runs/r-test" {
                    (format!("{{\"status\":\"{status}\"}}"), false, false)
                } else {
                    // Never let identity discovery accidentally select an unrelated Run.
                    ("{\"runs\":[{\"id\":\"unrelated\",\"status\":\"running\"}]}".into(), false, false)
                };
                let content_type = if sse { "text/event-stream" } else { "application/json" };
                let length = if keep_open { String::new() } else { format!("Content-Length: {}\r\n", body.len()) };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{length}Connection: close\r\n\r\n{body}");
                let _ = socket.write_all(response.as_bytes()).await;
                if keep_open { std::future::pending::<()>().await; }
            });
        }
    });
    (
        HttpTransport::new(format!("http://{address}"), String::new()).unwrap(),
        posts,
        cancels,
        server,
    )
}

#[tokio::test]
async fn clean_eof_and_done_marker_recover_the_same_ordered_terminal_trace() {
    for end in ["", "data: [DONE]\n\n"] {
        let replay = format!("{START}{TEXT}{}", done("error"));
        let (client, posts, _, server) =
            peer(format!("{START}{TEXT}{end}"), replay, "error", false, false).await;
        let seen = std::sync::Mutex::new(Vec::new());
        let messages = tokio::time::timeout(
            Duration::from_secs(2),
            client.start_run("a-test", "hello", None, |m| {
                if let Some(seq) = m.seq_id() {
                    seen.lock().unwrap().push(seq);
                }
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![0, 1, 2]);
        assert_eq!(messages.last().unwrap().data["status"], "error");
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

#[tokio::test]
async fn terminal_event_ends_observation_without_waiting_for_socket_eof() {
    let (client, _, _, server) = peer(
        format!("{START}{TEXT}{}", done("cancelled")),
        String::new(),
        "cancelled",
        true,
        false,
    )
    .await;
    let messages = tokio::time::timeout(
        Duration::from_millis(800),
        client.start_run("a-test", "hello", None, |_| {}),
    )
    .await
    .expect("run_done must terminate observation")
    .unwrap();
    assert_eq!(messages.last().unwrap().data["status"], "cancelled");
    server.abort();
}

#[tokio::test]
async fn status_only_recovery_delivers_a_terminal_outcome() {
    for status in ["done", "error", "cancelled"] {
        let (client, _, _, server) = peer(String::new(), String::new(), status, false, false).await;
        let messages = client.resume_run("r-test", 4, |_| {}).await.unwrap();
        let terminal = messages.last().unwrap();
        assert_eq!(terminal.msg_type(), "run_done");
        assert_eq!(terminal.data["status"], status);
        assert_eq!(terminal.run_id(), Some("r-test"));
        assert_eq!(
            terminal.seq_id(),
            None,
            "status evidence must not fabricate a log cursor"
        );
        server.abort();
    }
}

#[tokio::test]
async fn cancellation_before_identity_detaches_without_cancelling_an_unrelated_run() {
    let (client, posts, cancels, server) = peer(
        ": heartbeat\n\n".into(),
        String::new(),
        "running",
        true,
        false,
    )
    .await;
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        flag.store(true, Ordering::SeqCst);
    });
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        client.start_run_cancellable("a-test", "hello", None, |_| {}, Some(&cancel)),
    )
    .await
    .expect("cancellation must be bounded before identity")
    .unwrap_err();
    assert!(error.to_string().contains("unconfirmed"), "{error}");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(cancels.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn hanging_cancel_endpoint_detaches_with_the_accepted_identity() {
    let (client, posts, cancels, server) =
        peer(START.into(), String::new(), "running", true, true).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let error = tokio::time::timeout(
        Duration::from_secs(4),
        client.start_run_cancellable(
            "a-test",
            "hello",
            None,
            |m| {
                if m.run_id().is_some() {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            Some(&cancel),
        ),
    )
    .await
    .expect("cancel endpoint must not block observation")
    .unwrap_err();
    assert!(
        error.to_string().contains("r-test") && error.to_string().contains("unconfirmed"),
        "{error}"
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn terminal_cancellation_can_arrive_while_cancel_ack_is_hanging() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut live, _) = listener.accept().await.unwrap();
        let mut buf = [0; 8192];
        let _ = live.read(&mut buf).await.unwrap();
        live.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{START}").as_bytes()).await.unwrap();
        let (mut cancel, _) = listener.accept().await.unwrap();
        let n = cancel.read(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("POST /v1/runs/r-test/cancel "));
        live.write_all(format!("{TEXT}{}", done("cancelled")).as_bytes())
            .await
            .unwrap();
        // Neither the cancellation response nor live EOF is needed for the outcome.
        std::future::pending::<()>().await;
    });
    let client = HttpTransport::new(format!("http://{address}"), String::new()).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let messages = tokio::time::timeout(
        Duration::from_secs(1),
        client.start_run_cancellable(
            "a-test",
            "hello",
            None,
            |m| {
                if m.run_id().is_some() {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            Some(&cancel),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        cade_agent::agent::client::RunOutcome::from_messages(&messages).unwrap(),
        cade_agent::agent::client::RunOutcome::Cancelled
    );
    server.abort();
}

#[tokio::test]
async fn live_gap_and_duplicate_frames_replay_exactly_once() {
    let replay = format!("{START}{TEXT}{TEXT}{}", done("done"));
    let (client, posts, _, server) = peer(
        format!("{START}{START}{}", done("done")),
        replay,
        "done",
        false,
        false,
    )
    .await;
    let messages = client
        .start_run("a-test", "hello", None, |_| {})
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter_map(|m| m.seq_id())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn durable_gap_and_missing_identity_cannot_report_success() {
    let (client, _, _, server) = peer(START.into(), done("done"), "done", false, false).await;
    let error = client
        .start_run("a-test", "hello", None, |_| {})
        .await
        .unwrap_err();
    assert!(error.to_string().contains("journal gap"), "{error}");
    server.abort();
    let (client, posts, cancels, server) =
        peer(String::new(), String::new(), "done", false, false).await;
    let error = client
        .start_run("a-test", "hello", None, |_| {})
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unconfirmed"), "{error}");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(cancels.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn replay_uses_live_sse_framing_and_terminates_on_the_terminal_event() {
    let replay = format!(
        "{START}data:{{\"message_type\":\"assistant_message\",\n\
        data:\"run_id\":\"r-test\",\"seq_id\":1,\"content\":\"hello\"}}\n\n{}",
        done("done")
    );
    let (client, _, _, server) = peer(String::new(), replay, "done", true, false).await;
    let messages = tokio::time::timeout(
        Duration::from_millis(800),
        client.resume_run("r-test", -1, |_| {}),
    )
    .await
    .expect("replay must stop at run_done without EOF")
    .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter_map(|m| m.assistant_text())
            .collect::<String>(),
        "hello"
    );
    assert_eq!(
        messages
            .iter()
            .filter_map(|m| m.seq_id())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    server.abort();
}

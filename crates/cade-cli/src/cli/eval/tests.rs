use super::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct ServerState {
    requests: Vec<(String, String, Value)>,
    tasks: HashSet<String>,
    agents: BTreeMap<String, String>,
    runs: BTreeMap<String, (String, String)>,
    completions: Vec<Value>,
    workspaces: Vec<PathBuf>,
    fail_tools: bool,
    reject_completion: bool,
    config: Option<Value>,
    models: Option<Value>,
}

/// Exercise the actual HttpTransport/SSE adapter rather than replacing the
/// runner with a fake trait implementation. Each request has its own socket.
struct TestServer {
    client: HttpTransport,
    state: Arc<parking_lot::Mutex<ServerState>>,
    accept_loop: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

impl TestServer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(parking_lot::Mutex::new(ServerState::default()));
        let shared = state.clone();
        let accept_loop = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let state = shared.clone();
                tokio::spawn(async move {
                    serve_request(socket, state).await;
                });
            }
        });
        Self {
            client: HttpTransport::new(format!("http://{address}"), "eval-test-token".into())
                .unwrap(),
            state,
            accept_loop,
        }
    }
}

async fn serve_request(
    mut socket: tokio::net::TcpStream,
    state: Arc<parking_lot::Mutex<ServerState>>,
) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let size = socket.read(&mut buffer).await.unwrap();
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let mut request = headers.lines().next().unwrap().split_whitespace();
    let method = request.next().unwrap().to_owned();
    let path = request.next().unwrap().to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer eval-test-token")
    );
    while bytes.len() < header_end + length {
        let size = socket.read(&mut buffer).await.unwrap();
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..size]);
    }
    let body: Value = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    state
        .lock()
        .requests
        .push((method.clone(), path.clone(), body.clone()));

    if method == "POST" && path.ends_with("/run") {
        let agent_id = path.split('/').nth(3).unwrap().to_owned();
        let cwd = PathBuf::from(body["cwd"].as_str().expect("explicit isolated cwd"));
        assert_eq!(body["allowed_paths"], json!([cwd]));
        assert_eq!(body["execution"]["backend"], "local");
        assert_eq!(
            body["permission_mode"],
            PermissionMode::BypassPermissions.to_string()
        );
        assert!(cwd.join("fixture.txt").exists());
        let prompt = body["input"].as_str().unwrap();
        let run_id = format!("native-{agent_id}");
        {
            let mut state = state.lock();
            state.workspaces.push(cwd.clone());
            state
                .runs
                .insert(run_id.clone(), (agent_id, "running".into()));
        }
        // Lost initial handshake: cancellation must recover the accepted ID.
        if prompt == "lost-handshake" {
            send_json(
                &mut socket,
                400,
                &json!({"detail": "stream handshake lost"}),
            )
            .await;
            return;
        }
        if prompt != "timeout" {
            std::fs::write(cwd.join("agent.txt"), prompt).unwrap();
            let status = if prompt == "runtime-error" {
                "error"
            } else {
                "done"
            };
            state.lock().runs.get_mut(&run_id).unwrap().1 = status.into();
        }
        let mut events = vec![
            json!({"message_type":"stream_start", "run_id":run_id, "seq_id":0}),
            json!({"message_type":"usage_statistics", "run_id":run_id, "seq_id":1,
                "input_tokens":10, "output_tokens":3, "cache_read_tokens":4,
                "cache_write_tokens":2, "cost_usd":0.01, "model":"fixture/selected"}),
            json!({"message_type":"assistant_message", "run_id":run_id, "seq_id":2, "content":"verified answer"}),
        ];
        if prompt == "runtime-error" {
            events.push(json!({"message_type":"error", "run_id":run_id, "seq_id":3, "error":"provider failed"}));
        }
        if prompt != "timeout" {
            events.push(json!({"message_type":"run_done", "run_id":run_id,
                "seq_id":if prompt == "runtime-error" { 4 } else { 3 },
                "status":if prompt == "runtime-error" { "error" } else { "done" }}));
        }
        let mut response = String::from(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        );
        for event in events {
            response.push_str(&format!("data: {event}\n\n"));
        }
        if prompt != "timeout" {
            response.push_str("data: [DONE]\n\n");
        }
        if socket.write_all(response.as_bytes()).await.is_err() {
            return;
        }
        if prompt == "timeout" {
            // The runtime keeps working when its client stream is dropped.
            // Only the durable cancel route changes its status.
            for _ in 0..100 {
                if state.lock().runs[&run_id].1 == "cancelled" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        return;
    }

    let (status, response) = {
        let mut state = state.lock();
        match (method.as_str(), path.as_str()) {
            ("GET", "/v1/config") => (
                200,
                state
                    .config
                    .clone()
                    .unwrap_or_else(|| json!({"provider":"fixture", "default_model":"selected"})),
            ),
            ("GET", "/v1/models") => (
                200,
                state
                    .models
                    .clone()
                    .unwrap_or_else(|| json!({"dynamic":[{"id":"fixture/discovered"}]})),
            ),
            ("GET", "/v1/agents") => (
                200,
                json!(
                    state
                        .agents
                        .iter()
                        .map(|(id, name)| json!({"id":id,"name":name,"model":"fixture/selected"}))
                        .collect::<Vec<_>>()
                ),
            ),
            ("POST", "/v1/agents") => {
                let id = format!("agent-{}", state.agents.len() + state.requests.len());
                state
                    .agents
                    .insert(id.clone(), body["name"].as_str().unwrap().to_owned());
                (
                    200,
                    json!({"id":id,"name":body["name"],"model":body["model"]}),
                )
            }
            ("POST", "/v1/tools") if state.fail_tools => {
                (503, json!({"detail":"tool transport unavailable"}))
            }
            ("POST", "/v1/tools") => (
                200,
                json!({"id":"tool-1", "name":body["json_schema"]["name"]}),
            ),
            ("POST", "/v1/evals/tasks") => {
                assert!(body["expected_json"]["assertions"].is_array());
                let id = format!("task-{}", state.tasks.len());
                state.tasks.insert(id.clone());
                (200, json!({"id":id}))
            }
            ("POST", "/v1/evals/runs") => {
                assert!(
                    state.tasks.contains(body["task_id"].as_str().unwrap()),
                    "real task foreign key"
                );
                (200, json!({"id":format!("eval-{}", state.requests.len())}))
            }
            ("PATCH", _) if path.starts_with("/v1/evals/runs/") => {
                state.completions.push(body.clone());
                (200, json!({"updated":!state.reject_completion}))
            }
            ("POST", _) if path.ends_with("/tools") => (200, json!({"attached":1})),
            ("GET", _) if path.ends_with("/runs") => {
                let agent = path.split('/').nth(3).unwrap();
                (
                    200,
                    json!({"runs":state.runs.iter().filter(|(_, (id, _))| id == agent)
                    .map(|(id, (_, status))| json!({"id":id,"status":status})).collect::<Vec<_>>()}),
                )
            }
            ("POST", _) if path.ends_with("/cancel") => {
                let id = path.split('/').nth(3).unwrap();
                state
                    .runs
                    .get_mut(id)
                    .expect("cancel accepted durable run")
                    .1 = "cancelled".into();
                (200, json!({"id":id,"status":"cancelling"}))
            }
            ("GET", _) if path.starts_with("/v1/runs/") => {
                let id = path.split('/').nth(3).unwrap();
                (200, json!({"id":id,"status":state.runs[id].1}))
            }
            ("DELETE", _) if path.starts_with("/v1/agents/") => {
                let id = path.split('/').nth(3).unwrap();
                assert!(
                    state
                        .runs
                        .values()
                        .filter(|(agent, _)| agent == id)
                        .all(|(_, status)| status != "running"),
                    "cancel before deleting the agent"
                );
                state.agents.remove(id).expect("delete this task's agent");
                (200, json!({"deleted":true}))
            }
            _ => panic!("unexpected route {method} {path}"),
        }
    };
    send_json(&mut socket, status, &response).await;
}

async fn send_json(socket: &mut tokio::net::TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
}

fn task(prompt: &str) -> EvalTask {
    EvalTask {
        name: prompt.into(),
        description: None,
        prompt: prompt.into(),
        setup: None,
        assertions: vec![
            Assertion::OutputContains {
                text: "verified".into(),
            },
            Assertion::FileExists {
                path: "agent.txt".into(),
            },
        ],
        timeout_secs: 5,
    }
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fixture.txt"), "original fixture").unwrap();
    dir
}

#[tokio::test]
async fn successful_tasks_use_distinct_workspaces_and_persist_measurements() {
    let server = TestServer::new().await;
    let source = workspace();
    let first = task("one");
    let second = task("two");
    let (one, two) = tokio::join!(
        run_task(&server.client, &first, None, source.path()),
        run_task(&server.client, &second, None, source.path()),
    );
    for result in [one.unwrap(), two.unwrap()] {
        assert!(result.passed, "{:?}", result.failures);
        assert_eq!(result.score, 1.0);
        assert_eq!(result.model.as_deref(), Some("fixture/selected"));
        assert_eq!(result.stats.input_tokens, Some(10));
        assert_eq!(result.stats.output_tokens, Some(3));
        assert_eq!(result.stats.cache_read_tokens, Some(4));
        assert_eq!(result.stats.cache_write_tokens, Some(2));
        assert_eq!(result.stats.cost_usd, Some(0.01));
        assert!(result.stats.complete);
        assert!(result.persistence_ms.is_some());
        assert!(result.phase_ms.contains_key("runtime"));
        assert!(result.phase_ms.contains_key("verification"));
    }
    assert_eq!(
        std::fs::read_to_string(source.path().join("fixture.txt")).unwrap(),
        "original fixture"
    );
    assert!(!source.path().join("agent.txt").exists());
    let state = server.state.lock();
    assert!(state.agents.is_empty());
    assert_eq!(state.workspaces.len(), 2);
    assert_ne!(state.workspaces[0], state.workspaces[1]);
    assert!(state.workspaces.iter().all(|path| !path.exists()));
    assert_eq!(state.completions.len(), 2);
    for completion in &state.completions {
        assert_eq!(completion["status"], "passed");
        assert!(completion["completed_at"].as_i64().unwrap() > 0);
        let record: Value =
            serde_json::from_str(completion["result_json"].as_str().unwrap()).unwrap();
        assert_eq!(record["stats"]["input_tokens"], 10);
        assert_eq!(record["stats"]["cost_usd"], 0.01);
        assert!(
            record["runtime_run_id"]
                .as_str()
                .unwrap()
                .starts_with("native-")
        );
        assert!(
            record["persistence_ms"].is_null(),
            "completion request is not yet measured inside itself"
        );
    }
}

#[tokio::test]
async fn timeout_cancels_durable_work_and_keeps_partial_usage() {
    let server = TestServer::new().await;
    let source = workspace();
    let mut task = task("timeout");
    task.timeout_secs = 1;
    let result = run_task(
        &server.client,
        &task,
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert_eq!(result.status, "timed_out");
    assert_eq!(result.failed_phase.as_deref(), Some("runtime"));
    assert!(!result.passed);
    assert_eq!(result.score, 0.0);
    assert_eq!(result.stats.input_tokens, Some(10));
    assert!(!result.stats.complete);
    assert_eq!(result.output, "verified answer");
    let state = server.state.lock();
    assert!(state.agents.is_empty());
    assert!(state.runs.values().all(|(_, status)| status == "cancelled"));
    assert_eq!(state.completions[0]["status"], "timed_out");
    assert!(state.workspaces.iter().all(|path| !path.exists()));
}

#[tokio::test]
async fn lost_handshake_still_finds_and_stops_the_accepted_run() {
    let server = TestServer::new().await;
    let source = workspace();
    let mut task = task("lost-handshake");
    task.timeout_secs = 1;
    let result = run_task(
        &server.client,
        &task,
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert!(!result.passed);
    assert_eq!(result.score, 0.0);
    assert!(result.runtime_run_id.is_some());
    let state = server.state.lock();
    assert!(state.agents.is_empty());
    assert!(state.runs.values().all(|(_, status)| status == "cancelled"));
    assert_eq!(state.completions.len(), 1);
}

#[tokio::test]
async fn benchmark_keeps_invalid_and_runtime_failed_tasks_in_its_denominator() {
    let server = TestServer::new().await;
    let source = workspace();
    let tasks = tempfile::tempdir().unwrap();
    std::fs::write(tasks.path().join("01-invalid.json"), "not json").unwrap();
    let mut empty = task("empty-verifiers");
    empty.assertions.clear();
    for (file, task) in [
        ("02-empty.json", empty),
        ("03-runtime.json", task("runtime-error")),
        ("04-success.json", task("success")),
    ] {
        std::fs::write(
            tasks.path().join(file),
            serde_json::to_string(&task).unwrap(),
        )
        .unwrap();
    }
    let results = cmd_bench(
        &server.client,
        tasks.path(),
        Some("fixture/explicit"),
        2,
        source.path(),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 4);
    assert_eq!(results.iter().filter(|r| r.passed).count(), 1);
    assert_eq!(
        results.iter().map(|r| r.score).sum::<f64>() / results.len() as f64,
        0.25
    );
    assert_eq!(results[0].failed_phase.as_deref(), Some("load"));
    assert_eq!(results[1].failed_phase.as_deref(), Some("validation"));
    assert_eq!(results[2].failed_phase.as_deref(), Some("runtime"));
    assert!(server.state.lock().agents.is_empty());
    assert!(
        cmd_bench(&server.client, tasks.path(), None, 0, source.path())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn transport_setup_and_storage_failures_are_explicit_results() {
    let server = TestServer::new().await;
    let source = workspace();
    server.state.lock().fail_tools = true;
    let transport = run_task(
        &server.client,
        &task("transport"),
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert_eq!(transport.failed_phase.as_deref(), Some("tools"));
    assert_eq!(transport.score, 0.0);
    assert!(server.state.lock().agents.is_empty());
    assert_eq!(server.state.lock().completions[0]["status"], "error");

    let mut setup = task("setup");
    setup.setup = Some("exit 7".into());
    let setup = run_task(&server.client, &setup, None, source.path())
        .await
        .unwrap();
    assert_eq!(setup.failed_phase.as_deref(), Some("setup"));
    assert_eq!(setup.score, 0.0);
    assert!(setup.failures.iter().any(|failure| failure.contains('7')));
    assert!(setup.agent_id.is_none());

    {
        let mut state = server.state.lock();
        state.fail_tools = false;
        state.reject_completion = true;
    }
    let storage = run_task(
        &server.client,
        &task("storage"),
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert_eq!(storage.failed_phase.as_deref(), Some("persistence"));
    assert!(!storage.passed);
    assert_eq!(storage.score, 0.0);
    assert!(server.state.lock().agents.is_empty());
}

#[tokio::test]
async fn failed_verifiers_preserve_partial_score_in_the_completed_record() {
    let server = TestServer::new().await;
    let source = workspace();
    let mut task = task("partial-score");
    task.assertions.push(Assertion::OutputContains {
        text: "missing marker".into(),
    });
    let result = run_task(
        &server.client,
        &task,
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert!(!result.passed);
    assert_eq!(result.status, "failed");
    assert_eq!(result.failed_phase.as_deref(), Some("verification"));
    assert_eq!(result.score, 2.0 / 3.0);
    assert_eq!(result.failures.len(), 1);
    assert!(result.stats.complete);
    assert_eq!(
        server.state.lock().completions[0]["score"].as_f64(),
        Some(2.0 / 3.0)
    );
    assert!(server.state.lock().agents.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn verification_timeout_is_measured_and_does_not_leave_an_ephemeral_agent() {
    let server = TestServer::new().await;
    let source = workspace();
    let mut task = task("verification-timeout");
    task.timeout_secs = 1;
    task.assertions.push(Assertion::CommandPasses {
        command: "exec sleep 5".into(),
    });
    let result = run_task(
        &server.client,
        &task,
        Some("fixture/explicit"),
        source.path(),
    )
    .await
    .unwrap();
    assert_eq!(result.status, "timed_out");
    assert_eq!(result.failed_phase.as_deref(), Some("verification"));
    assert_eq!(result.score, 0.0);
    assert!(result.phase_ms["verification"] > 0);
    assert!(
        result.stats.complete,
        "the native run ended before verification timed out"
    );
    assert_eq!(result.stats.input_tokens, Some(10));
    assert!(server.state.lock().agents.is_empty());
    assert_eq!(server.state.lock().completions[0]["status"], "timed_out");
    assert!(!source.path().join("agent.txt").exists());
}

#[tokio::test]
async fn verifiers_read_only_the_task_copy_and_run_commands_there() {
    let source = workspace();
    let copy = IsolatedWorkspace::clone_from(source.path()).unwrap();
    std::fs::write(copy.path().join("agent.txt"), "child-only").unwrap();
    let assertions = vec![
        Assertion::FileExists {
            path: "agent.txt".into(),
        },
        Assertion::FileNotExists {
            path: "absent.txt".into(),
        },
        Assertion::CommandPasses {
            command: "exit 0".into(),
        },
        Assertion::OutputContains {
            text: "expected".into(),
        },
        Assertion::OutputNotContains {
            text: "forbidden".into(),
        },
    ];
    assert!(
        verify_assertions(&assertions, "expected", copy.path())
            .await
            .is_empty()
    );
    assert_eq!(
        verify_assertions(&assertions, "expected", source.path())
            .await
            .len(),
        1
    );
    assert_eq!(
        verify_assertions(
            &[Assertion::CommandPasses {
                command: "exit 9".into()
            }],
            "",
            copy.path()
        )
        .await
        .len(),
        1
    );
    assert_eq!(verify_assertions(&[], "", copy.path()).await.len(), 1);
    drop(copy);
    assert!(!source.path().join("agent.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn setup_mutations_stay_in_the_task_copy() {
    let server = TestServer::new().await;
    let source = workspace();
    let mut task = task("setup-isolated");
    task.setup = Some("printf changed > fixture.txt".into());
    task.assertions.push(Assertion::CommandPasses {
        command: "test \"$(cat fixture.txt)\" = changed".into(),
    });
    assert!(
        run_task(
            &server.client,
            &task,
            Some("fixture/explicit"),
            source.path()
        )
        .await
        .unwrap()
        .passed
    );
    assert_eq!(
        std::fs::read_to_string(source.path().join("fixture.txt")).unwrap(),
        "original fixture"
    );
}

#[test]
fn task_validation_rejects_vacuous_and_escaping_verifiers() {
    let mut task = task("validation");
    for assertions in [
        vec![],
        vec![Assertion::OutputContains { text: "".into() }],
        vec![Assertion::CommandPasses {
            command: "  ".into(),
        }],
        vec![Assertion::FileNotExists {
            path: "../outside".into(),
        }],
    ] {
        task.assertions = assertions;
        assert!(validate_task(&task).is_err());
    }
    assert!(validate_relative_path(".").is_err());
    assert!(validate_relative_path("nested/output.txt").is_ok());
}

#[cfg(unix)]
#[test]
fn file_verifiers_reject_symlinks_outside_the_task_copy() {
    let source = workspace();
    let outside = workspace();
    std::os::unix::fs::symlink(outside.path(), source.path().join("external")).unwrap();
    assert!(verifier_file_exists(source.path(), "external/fixture.txt").is_err());
    assert!(verifier_file_exists(source.path(), "external/absent.txt").is_err());
}

#[test]
fn usage_is_deduplicated_and_missing_cost_remains_unknown() {
    let mut capture = RunCapture::default();
    let event = |value| serde_json::from_value::<CadeMessage>(value).unwrap();
    let usage = event(
        json!({"message_type":"usage_statistics", "run_id":"run", "seq_id":2,
        "input_tokens":12, "output_tokens":4, "cache_read_tokens":2, "unrecognized_native_stat":42}),
    );
    capture.observe(&usage);
    capture.observe(&usage);
    capture.observe(&event(
        json!({"message_type":"usage_statistics", "run_id":"run", "seq_id":3,
        "input_tokens":7, "output_tokens":3, "cache_read_tokens":1}),
    ));
    let mut result = EvalResult::new("accounting".into());
    capture.finish(&mut result);
    assert_eq!(result.stats.input_tokens, Some(19));
    assert_eq!(result.stats.output_tokens, Some(7));
    assert_eq!(result.stats.cache_read_tokens, Some(3));
    assert_eq!(result.stats.cache_write_tokens, None);
    assert_eq!(result.stats.cost_usd, None);
    assert!(!result.stats.complete);
    assert_eq!(result.stats.usage_events.len(), 2);
    assert_eq!(result.stats.usage_events[0]["unrecognized_native_stat"], 42);
    assert!(result.stats.usage_source.is_some());
}

#[test]
fn configured_models_preserve_qualified_ids_without_inventing_defaults() {
    assert_eq!(
        configured_model(&json!({"provider":"fixture","default_model":"selected"})),
        Some("fixture/selected".into())
    );
    assert_eq!(
        configured_model(&json!({"provider":"ignored","default_model":"custom/qualified"})),
        Some("custom/qualified".into())
    );
    assert_eq!(
        configured_model(&json!({"provider":"fixture","default_model":""})),
        None
    );
    assert_eq!(configured_model(&json!({})), None);
    assert_eq!(
        configured_model(
            &json!({"provider":"unavailable", "default_model":"selected",
        "available_providers":["fixture"]})
        ),
        None
    );
}

#[tokio::test]
async fn unavailable_default_uses_discovery_and_empty_discovery_is_an_error() {
    let server = TestServer::new().await;
    server.state.lock().config = Some(json!({"provider":"unavailable", "default_model":"selected",
        "available_providers":["fixture"]}));
    assert_eq!(
        server_model(&server.client).await.unwrap(),
        "fixture/discovered"
    );
    server.state.lock().models = Some(json!({"dynamic":[]}));
    assert!(server_model(&server.client).await.is_err());
}

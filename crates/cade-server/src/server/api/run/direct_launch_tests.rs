//! HTTP contracts for the direct-launch inspection run's authoritative outcome.
use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cade_ai::{CompletionResponse, LlmProvider};
use std::{pin::Pin, sync::Arc};
use tower::ServiceExt;

type ModelStream = Pin<Box<dyn futures::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>;

fn parent_state(provider: Arc<dyn LlmProvider>) -> AppState {
    let mut state = super::tests::build_state_with_llm(provider);
    Arc::make_mut(&mut state.config).api_key = Some("direct-launch-test-token".into());
    sqlite::create_agent(
        &state.db,
        &sqlite::AgentRow {
            id: "direct-parent".into(),
            name: "Direct launch parent".into(),
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
    state
}

async fn launch_http(state: AppState, args: Value) -> Value {
    let response = crate::server::api::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/agents/direct-parent/subagents")
                .header("Authorization", "Bearer direct-launch-test-token")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({"mode":"default", "args":args}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn wait_terminal(state: &AppState, run_id: &str, expected: &str) -> Vec<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = sqlite::get_run(&state.db, run_id).unwrap().unwrap();
            if matches!(run.status.as_str(), "done" | "error" | "cancelled") {
                assert_eq!(run.status, expected);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("explicit execution outcome must finalize the direct run");
    let events: Vec<Value> = sqlite::run_events_after(&state.db, run_id, -1)
        .unwrap()
        .into_iter()
        .map(|(_, data)| serde_json::from_str(&data).unwrap())
        .collect();
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| event["message_type"] == "run_done")
        .collect();
    assert_eq!(
        terminal.len(),
        1,
        "exactly one durable run terminal: {events:?}"
    );
    assert_eq!(terminal[0]["status"], expected);
    assert_eq!(terminal[0]["run_id"], run_id);
    assert!(
        events
            .iter()
            .filter(|event| event["message_type"] == "subagent_complete")
            .count()
            <= 1
    );

    let response = crate::server::api::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/v1/runs/{run_id}"))
                .header("Authorization", "Bearer direct-launch-test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resource: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resource["status"], expected);
    events
}

struct TextProvider;
#[async_trait::async_trait]
impl LlmProvider for TextProvider {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        Ok(CompletionResponse {
            content: Some(
                "Success summary mentioning cancelled and error as ordinary words".into(),
            ),
            tool_calls: vec![],
            finish_reason: "stop".into(),
        })
    }
    async fn stream(&self, _: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        unreachable!()
    }
}

#[tokio::test]
async fn direct_http_success_persists_one_terminal_outcome_before_response() {
    let state = parent_state(Arc::new(TextProvider));
    let response = launch_http(state.clone(), json!({"prompt":"work"})).await;
    assert_eq!(response["is_error"], false, "{response}");
    let run_id = response["run_id"].as_str().unwrap();
    assert_eq!(
        sqlite::get_run(&state.db, run_id).unwrap().unwrap().status,
        "done"
    );
    let events = wait_terminal(&state, run_id, "done").await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event["message_type"] == "subagent_complete")
            .count(),
        1
    );
    assert!(state.subagent_cancellations.read().await.is_empty());
}

#[tokio::test]
async fn direct_http_invalid_definition_is_error_even_without_child_terminal_event() {
    for background in [false, true] {
        let state = parent_state(Arc::new(super::tests::PanicOnCallLlm));
        let response = launch_http(
            state.clone(),
            json!({
                "prompt":"work", "mode":"missing-direct-definition", "background":background,
            }),
        )
        .await;
        assert_eq!(response["is_error"], true, "{response}");
        let run_id = response["run_id"].as_str().unwrap();
        assert_eq!(
            sqlite::get_run(&state.db, run_id).unwrap().unwrap().status,
            "error"
        );
        let events = wait_terminal(&state, run_id, "error").await;
        assert!(
            events.last().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("missing-direct-definition")
        );
        assert!(
            !events
                .iter()
                .any(|event| event["message_type"] == "subagent_complete")
        );
        assert!(state.subagent_cancellations.read().await.is_empty());
        assert_eq!(sqlite::list_agents(&state.db).unwrap().len(), 1);
    }
}

#[tokio::test]
async fn direct_http_closed_admission_persists_error_for_sync_and_background() {
    for background in [false, true] {
        let state = parent_state(Arc::new(super::tests::PanicOnCallLlm));
        state.subagent_semaphore.close();
        let response = launch_http(
            state.clone(),
            json!({"prompt":"work", "background":background}),
        )
        .await;
        assert_eq!(
            response["is_error"], !background,
            "acknowledgement and terminal outcome differ: {response}"
        );
        let events = wait_terminal(&state, response["run_id"].as_str().unwrap(), "error").await;
        assert!(
            events
                .iter()
                .any(|event| event.to_string().to_lowercase().contains("closed")),
            "{events:?}"
        );
        assert!(state.subagent_cancellations.read().await.is_empty());
        assert_eq!(sqlite::list_agents(&state.db).unwrap().len(), 1);
    }
}

struct BlockedProvider {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl LlmProvider for BlockedProvider {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(CompletionResponse {
            content: Some("background finished".into()),
            tool_calls: vec![],
            finish_reason: "stop".into(),
        })
    }
    async fn stream(&self, _: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        unreachable!()
    }
}

#[tokio::test]
async fn direct_http_background_acknowledgement_stays_running_until_real_success() {
    let provider = Arc::new(BlockedProvider {
        entered: Default::default(),
        release: Default::default(),
    });
    let state = parent_state(provider.clone());
    let response = launch_http(state.clone(), json!({"prompt":"work", "background":true})).await;
    assert_eq!(response["is_error"], false);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        provider.entered.notified(),
    )
    .await
    .unwrap();
    let run_id = response["run_id"].as_str().unwrap();
    assert_eq!(
        sqlite::get_run(&state.db, run_id).unwrap().unwrap().status,
        "running"
    );
    assert!(
        !sqlite::run_events_after(&state.db, run_id, -1)
            .unwrap()
            .iter()
            .any(|(_, data)| data.contains("run_done"))
    );
    provider.release.notify_one();
    let events = wait_terminal(&state, run_id, "done").await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event["message_type"] == "subagent_complete")
            .count(),
        1
    );
}

#[tokio::test]
async fn direct_http_cancellation_uses_session_outcome_and_persists_one_terminal() {
    for background in [false, true] {
        let provider = Arc::new(BlockedProvider {
            entered: Default::default(),
            release: Default::default(),
        });
        let state = parent_state(provider.clone());
        let launch = tokio::spawn(launch_http(
            state.clone(),
            json!({"prompt":"work", "background":background}),
        ));
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            provider.entered.notified(),
        )
        .await
        .unwrap();
        let run_id = sqlite::list_agent_runs(&state.db, "direct-parent", 10).unwrap()[0]
            .id
            .clone();
        let child_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(id) = sqlite::run_events_after(&state.db, &run_id, -1)
                    .unwrap()
                    .into_iter()
                    .filter_map(|(_, data)| serde_json::from_str::<Value>(&data).ok())
                    .find(|event| event["message_type"] == "subagent_started")
                    .and_then(|event| event["subagent_id"].as_str().map(str::to_owned))
                {
                    break id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let cancel = crate::server::api::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/subagents/{child_id}/cancel"))
                    .header("Authorization", "Bearer direct-launch-test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cancel.status(), StatusCode::OK);
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), launch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response["is_error"], !background);
        let events = wait_terminal(&state, &run_id, "cancelled").await;
        let child_terminal: Vec<_> = events
            .iter()
            .filter(|event| event["message_type"] == "subagent_complete")
            .collect();
        assert_eq!(child_terminal.len(), 1);
        assert_eq!(child_terminal[0]["status"], "cancelled");
        assert!(state.subagent_cancellations.read().await.is_empty());
        assert_eq!(state.subagent_semaphore.available_permits(), 4);
        assert_eq!(sqlite::list_agents(&state.db).unwrap().len(), 1);
    }
}

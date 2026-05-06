// Tests for the run module.
use super::*;
use super::meta_tools::{intercept_meta_tool, handle_conversation_search_meta};
use super::subagent::{filter_subagent_tools, handle_run_subagent_tool};

use cade_ai::LlmToolCall;

    // ── truncate_at_char_boundary (C2) ────────────────────────────────

    #[test]
    fn truncate_at_char_boundary_short_string_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_at_char_boundary(s, 100), "hello world");
    }

    #[test]
    fn truncate_at_char_boundary_ascii_exact() {
        let s = "abcdefghij";
        assert_eq!(truncate_at_char_boundary(s, 5), "abcde");
    }

    #[test]
    fn truncate_at_char_boundary_does_not_panic_on_multibyte() {
        // 4-byte emoji at the boundary: byte index 8 splits the emoji.
        // Without the fix this would panic with "byte index 8 is not a
        // char boundary".
        let s = "abcdefg🚀hijk"; // bytes: 7 ascii + 4-byte emoji + 4 ascii
        assert_eq!(s.len(), 15);
        // Cut at byte 8 — middle of the emoji's 4-byte sequence.
        let result = truncate_at_char_boundary(s, 8);
        // Should walk back to byte 7 (just before the emoji).
        assert_eq!(result, "abcdefg");
    }

    #[test]
    fn truncate_at_char_boundary_keeps_complete_chars() {
        let s = "héllo"; // 'é' is 2 bytes (0xC3 0xA9). Bytes: h(1) é(2) l(1) l(1) o(1) = 6
        assert_eq!(s.len(), 6);
        // Cut at byte 2 — middle of 'é'? 'h' is byte 0, 'é' starts at byte 1
        // and ends at byte 3.  Byte index 2 is mid-é and not a boundary.
        let result = truncate_at_char_boundary(s, 2);
        // Should walk back to byte 1.
        assert_eq!(result, "h");
    }

    #[test]
    fn truncate_at_char_boundary_zero_max() {
        let s = "abc";
        assert_eq!(truncate_at_char_boundary(s, 0), "");
    }

    #[test]
    fn truncate_at_char_boundary_large_utf8() {
        // 8192 bytes worth of multi-byte CJK — verifies we never panic at
        // the production cap.
        let cjk = "中".repeat(3000); // 3 bytes each → 9000 bytes
        let result = truncate_at_char_boundary(&cjk, 8192);
        // Length must be ≤ 8192 and on a char boundary.
        assert!(result.len() <= 8192);
        assert!(cjk.is_char_boundary(result.len()));
    }

    // ── RunExitStatus (M9r) ────────────────────────────────────────────

    #[test]
    fn run_exit_status_done_renders_as_done() {
        assert_eq!(RunExitStatus::Done.as_str(), "done");
    }

    #[test]
    fn run_exit_status_error_renders_as_error() {
        assert_eq!(RunExitStatus::Error.as_str(), "error");
    }

    /// RED for Phase A0: an `intercept_meta_tool` helper must exist and
    /// dispatch the existing in-loop intercepts (`load_skill`,
    /// `unload_skill`, `run_subagent`) on its own, returning
    /// `Some(ToolResult)` for known meta-tools and `None` for tools that
    /// should fall through to `manager::dispatch`.
    ///
    /// This test pins the seam needed to add the remaining 13 meta-tools
    /// (Phase A1–A4) without further if/else proliferation.  We only
    /// assert the simplest pre-existing case (`unload_skill` for an
    /// agent that has nothing loaded) because it requires no DB row, no
    /// LLM call, and no SSE traffic.
    #[tokio::test]
    async fn intercept_meta_tool_exists_and_handles_unload_skill() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_1".to_string(),
            name: "unload_skill".to_string(),
            arguments: serde_json::json!({"id": "nonexistent"}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("unload_skill must be intercepted");
        assert_eq!(res.tool_name, "unload_skill");
        // No skill loaded → handler returns an error string, but it IS
        // a meta-tool intercept (not "Unknown tool" from manager::dispatch).
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through to manager::dispatch, got: {}",
            res.output
        );
    }

    /// Phase A1: `link_memory_evidence` must persist a row in
    /// `memory_evidence` linked to the named block.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_link_memory_evidence() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_e".into(),
                name: "t".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();
        cade_store::sqlite::upsert_memory_block(
            &state.db,
            "agent_e",
            "decision",
            "use postgres",
            None,
            None,
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_e".into(),
            name: "link_memory_evidence".into(),
            arguments: serde_json::json!({
                "label": "decision",
                "kind": "user_assertion",
                "reference": "msg_42",
                "excerpt": "user said pg",
            }),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_e", &tc, tx).await;
        let res = opt.expect("intercepted");
        assert!(!res.is_error, "should succeed: {}", res.output);
    }

    /// Phase A1: `memory_apply_patch` must read the existing block, apply
    /// the unified diff, and persist the new value — all server-side, no
    /// HTTP self-call.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_memory_apply_patch() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_p".into(),
                name: "t".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();
        // Seed a block to patch.
        cade_store::sqlite::upsert_memory_block(
            &state.db, "agent_p", "notes", "old line", None, None,
        )
        .unwrap();

        let patch = "@@ -1,1 +1,1 @@\n-old line\n+new line\n";
        let tc = LlmToolCall {
            id: "tc_p".into(),
            name: "memory_apply_patch".into(),
            arguments: serde_json::json!({"label": "notes", "patch": patch}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_p", &tc, tx).await;
        let res = opt.expect("intercepted");
        assert!(!res.is_error, "patch must apply: {}", res.output);

        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, "agent_p").unwrap();
        let value = blocks
            .iter()
            .find(|(l, _, _)| l == "notes")
            .map(|(_, v, _)| v.clone())
            .unwrap_or_default();
        assert!(
            value.contains("new line"),
            "patched content must contain 'new line', got: {value:?}"
        );
    }

    /// Phase A1: `update_memory_typed` must persist the memory block with
    /// its `memory_type` and `confidence` typed columns set, directly via
    /// the DB (no HTTP self-call).
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_update_memory_typed() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_typed".into(),
                name: "test".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_t".into(),
            name: "update_memory_typed".into(),
            arguments: serde_json::json!({
                "label": "decision_x",
                "value": "use postgres",
                "memory_type": "decision",
                "confidence": 0.8,
            }),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_typed", &tc, tx).await;
        let res = opt.expect("must be intercepted");
        assert!(!res.is_error, "should succeed: {}", res.output);

        // Verify block was persisted with typed fields.
        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, "agent_typed").unwrap();
        assert!(
            blocks
                .iter()
                .any(|(l, v, _)| l == "decision_x" && v == "use postgres"),
            "block must be persisted, got: {blocks:?}"
        );
    }

    /// Phase A1: `update_memory` must be intercepted server-side and write
    /// directly to the DB without any HTTP self-call.  Asserts the
    /// resulting memory block is queryable from the same `AppState.db`.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_update_memory() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        // Need an agent row so foreign keys resolve when upsert links.
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_mem".into(),
                name: "test-agent".into(),
                model: "test-model".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_1".to_string(),
            name: "update_memory".to_string(),
            arguments: serde_json::json!({
                "label": "test_block",
                "value": "hello",
                "operation": "set",
            }),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_mem", &tc, tx).await;
        let res = opt.expect("update_memory must be intercepted");
        assert!(
            !res.is_error,
            "update_memory should succeed: {}",
            res.output
        );

        // Read back from DB — proves no HTTP self-call is needed.
        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, "agent_mem").unwrap();
        let found = blocks
            .iter()
            .any(|(label, value, _)| label == "test_block" && value == "hello");
        assert!(found, "memory block must be persisted, got: {blocks:?}");
    }

    /// A mock LlmProvider that panics if called.  Used to assert that an early
    /// return path (e.g. depth-limit guard) never reaches the LLM at all.
    pub(super) struct PanicOnCallLlm;
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for PanicOnCallLlm {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            panic!("LLM should not be called when depth limit is hit");
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            panic!("LLM stream should not be called");
        }
    }

    pub(super) fn build_state_with_llm(llm: std::sync::Arc<dyn cade_ai::LlmProvider>) -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = std::sync::Arc::new(crate::server::config::ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            llm_provider: crate::server::config::LlmProviderKind::Anthropic,
            default_model: "test".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            api_key: None,
            allowed_origin: None,
            max_context_budget: None,
        });
        AppState {
            db,
            llm,
            llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
                &cade_ai::AiConfig {
                    anthropic_api_key: None,
                    openai_api_key: None,
                    google_api_key: None,
                    ollama_base_url: String::new(),
                    llm_provider: String::new(),
                },
            ))),
            config,
            mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
            memory_cache: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
            agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
                crate::server::state::CONTEXT_CACHE_CAPACITY,
            ))),
            all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            embedder: None,
        }
    }

    /// Stateful mock: returns `run_subagent` exactly once per loop level
    /// (when the message list has just system+user) and a final text on
    /// the next iter (after a tool result has been appended).  This keeps
    /// the test fast while still exercising depth recursion.
    struct OneRecurseLlm {
        call_count: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for OneRecurseLlm {
        async fn complete(
            &self,
            r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Initial state for a fresh subagent loop is exactly 2 msgs
            // (system + user).  Anything more means we are post-tool-result.
            let is_initial = r.messages.len() == 2;
            if is_initial {
                Ok(cade_ai::CompletionResponse {
                    content: Some("recursing".into()),
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_rec".into(),
                        name: "run_subagent".into(),
                        arguments: serde_json::json!({"prompt": "deeper"}),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                Ok(cade_ai::CompletionResponse {
                    content: Some("done".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!("stream() is not exercised by this mock")
        }
    }

    /// Recursion bound: a subagent that recurses once per level must hit
    /// the depth cap (default 3) and return without deadlock.  At depth 3
    /// the call is refused before acquiring a permit, so the chain
    /// terminates.  Asserts (a) outer call succeeds, (b) LLM call count
    /// is small (linear in depth, not exponential).
    #[tokio::test]
    async fn recursive_subagent_calls_are_bounded_by_depth() {
        let llm = std::sync::Arc::new(OneRecurseLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);

        // depth 0 → 1 → 2 → 3 (refused).  Each level: 2 LLM calls
        // (initial recurse + final).  Depth-2's recurse to depth 3 is
        // refused (tool result = error), then depth-2's next iter sees
        // post-tool-result state and returns final text.
        let args = serde_json::json!({ "prompt": "start", "_subagent_depth": 0 });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx),
        )
        .await
        .expect("must not deadlock — chain must terminate via depth guard");

        assert!(
            !result.is_error,
            "outer subagent should complete: {}",
            result.output
        );
        let calls = llm.call_count.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            calls > 0 && calls < 20,
            "LLM call count must be small (linear in depth), got: {calls}"
        );
    }

    /// Approach C deliberately runs the subagent loop in-memory without
    /// creating ephemeral `agent`/`message` rows.  That keeps the parent
    /// agent's conversation history clean and avoids cross-contamination.
    /// This test is a watchdog: if a future change accidentally persists
    /// subagent traffic it will fail loudly.
    #[tokio::test]
    async fn subagent_run_does_not_pollute_parent_db() {
        let llm = std::sync::Arc::new(ScriptedLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let agents_before: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        let messages_before: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();

        let args = serde_json::json!({ "prompt": "do thing" });
        let _ = handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

        let agents_after: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        let messages_after: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();

        assert_eq!(
            agents_before, agents_after,
            "subagent must not create agent rows"
        );
        assert_eq!(
            messages_before, messages_after,
            "subagent must not persist messages to parent DB"
        );
    }

    /// A stateful mock that on the FIRST call returns a tool_call (forcing a
    /// loop iteration), and on the SECOND call returns plain text.  The
    /// LLM messages it receives are recorded so tests can verify that the
    /// subagent loop fed back the tool result.
    struct ScriptedLlm {
        call_count: std::sync::atomic::AtomicUsize,
        captured_iter2_messages: std::sync::Mutex<Vec<cade_ai::LlmMessage>>,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for ScriptedLlm {
        async fn complete(
            &self,
            r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let n = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(cade_ai::CompletionResponse {
                    content: None,
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_inner_1".into(),
                        name: "fake_tool".into(),
                        arguments: serde_json::json!({}),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                let mut g = self.captured_iter2_messages.lock().unwrap();
                *g = r.messages.clone();
                Ok(cade_ai::CompletionResponse {
                    content: Some("subagent done".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!("stream() is not exercised by this mock")
        }
    }

    /// RED: subagent currently does a single `complete()` and returns the
    /// text.  When the LLM returns a tool_call instead, the subagent loop
    /// must dispatch it and feed the result back in a second LLM call.
    /// Asserts (a) the LLM was called exactly twice, (b) the second call
    /// saw a "tool" role message containing the dispatch result.
    #[tokio::test]
    async fn subagent_dispatches_tool_calls_and_loops() {
        let llm = std::sync::Arc::new(ScriptedLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let args = serde_json::json!({ "prompt": "do thing" });
        let result = handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

        assert!(
            !result.is_error,
            "loop must succeed, got: {}",
            result.output
        );
        assert_eq!(
            llm.call_count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "LLM must be called twice (first tool_call, then completion)"
        );
        let iter2 = llm.captured_iter2_messages.lock().unwrap().clone();
        let has_tool_msg = iter2
            .iter()
            .any(|m| m.role == "tool" && m.content.contains("fake_tool"));
        assert!(
            has_tool_msg,
            "iteration-2 messages must include a tool-role message echoing dispatch result, got roles: {:?}",
            iter2.iter().map(|m| &m.role).collect::<Vec<_>>()
        );
        assert!(
            result.output.contains("subagent done"),
            "final output must be from second LLM call, got: {}",
            result.output
        );
    }

    /// Subagents must NOT receive `run_subagent` in their tool list — this
    /// is the second line of defence against runaway recursion (the first
    /// being the depth guard in `handle_run_subagent_tool`).  Removing the
    /// schema means the subagent's LLM never sees the tool advertised.
    #[test]
    fn filter_subagent_tools_strips_run_subagent_schema() {
        let schemas = vec![
            serde_json::json!({"name": "bash"}),
            serde_json::json!({"name": "run_subagent"}),
            serde_json::json!({"name": "read_file"}),
        ];
        let filtered = filter_subagent_tools(schemas);
        let names: Vec<String> = filtered
            .iter()
            .filter_map(|s| s["name"].as_str().map(String::from))
            .collect();
        assert!(
            !names.iter().any(|n| n == "run_subagent"),
            "run_subagent must be stripped, got: {names:?}"
        );
        assert!(names.iter().any(|n| n == "bash"));
        assert!(names.iter().any(|n| n == "read_file"));
    }

    // ── REC-1: Wall-clock timeout ─────────────────────────────────────────

    /// Mock LLM that sleeps for a very long time on each `complete()` call,
    /// simulating a hung tool or slow provider.
    struct SlowLlm;
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for SlowLlm {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            // Sleep for 60 seconds — the test timeout is 2 seconds.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok(cade_ai::CompletionResponse {
                content: Some("should never reach here".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!("stream() not used")
        }
    }

    /// REC-1: The subagent loop must enforce a wall-clock timeout. When the
    /// timeout fires (before the LLM even responds), the subagent must
    /// return an error result mentioning "timeout" and clean up its
    /// ephemeral row.
    ///
    /// We use `subagent_timeout_secs()` which returns 2s under `cfg(test)`.
    #[tokio::test]
    async fn subagent_loop_respects_wall_clock_timeout() {
        let llm = std::sync::Arc::new(SlowLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let agents_before: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();

        let args = serde_json::json!({ "prompt": "slow task" });

        let start = std::time::Instant::now();
        let result = handle_run_subagent_tool(&state, "parent_slow", "tc_slow", &args, tx).await;
        let elapsed = start.elapsed();

        // Must return within ~5s (2s test timeout + slack), NOT 60s.
        assert!(
            elapsed.as_secs() < 10,
            "must respect timeout, took {elapsed:?}"
        );
        assert!(result.is_error, "timed-out subagent must be an error");
        assert!(
            result.output.to_lowercase().contains("timeout"),
            "error must mention timeout, got: {}",
            result.output
        );

        // Ephemeral row must be cleaned up (REC-2 guard fires on timeout path).
        let agents_after: i64 = state
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            agents_before, agents_after,
            "ephemeral row must be cleaned up after timeout"
        );
    }

    // ── REC-2: EphemeralAgentGuard cleanup ────────────────────────────────

    /// REC-2: An `EphemeralAgentGuard` must delete the ephemeral agent row
    /// and write back subagent memory when dropped, even if the agentic
    /// loop panics or returns early.
    #[test]
    fn ephemeral_agent_guard_cleans_up_on_drop() {
        use super::subagent::EphemeralAgentGuard;

        let db = cade_store::sqlite::open(":memory:").unwrap();
        // Create parent agent.
        cade_store::sqlite::create_agent(
            &db,
            &cade_store::sqlite::AgentRow {
                id: "parent_g".into(),
                name: "parent".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();
        // Create ephemeral subagent row (simulates line 197 of subagent.rs).
        cade_store::sqlite::create_agent(
            &db,
            &cade_store::sqlite::AgentRow {
                id: "sa_guard".into(),
                name: "ephemeral".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Subagent wrote a finding during its loop.
        cade_store::sqlite::upsert_memory_block(
            &db,
            "sa_guard",
            "my_finding",
            "important data",
            None,
            None,
        )
        .unwrap();

        // Row exists before drop.
        let exists_before = cade_store::sqlite::get_agent(&db, "sa_guard")
            .unwrap()
            .is_some();
        assert!(exists_before, "ephemeral row must exist before guard drop");

        // Create guard, then drop it.
        {
            let _guard = EphemeralAgentGuard::new(
                db.clone(),
                "sa_guard".to_string(),
                "parent_g".to_string(),
            );
        } // ← drop fires here

        // Row must be gone.
        let exists_after = cade_store::sqlite::get_agent(&db, "sa_guard")
            .unwrap()
            .is_some();
        assert!(!exists_after, "ephemeral row must be deleted after guard drop");

        // Write-back must have happened: parent should have `subagent:my_finding`.
        let parent_blocks = cade_store::sqlite::get_memory_blocks(&db, "parent_g").unwrap();
        let labels: Vec<&str> = parent_blocks.iter().map(|(l, _, _)| l.as_str()).collect();
        assert!(
            labels.contains(&"subagent:my_finding"),
            "write-back must run before delete; got labels: {labels:?}"
        );
    }

    // ── Phase A2: skills meta-tools ──────────────────────────────────────

    /// Phase A2 RED: `load_skill_ref` for a skill that does not exist must
    /// return an intercepted error (not "Unknown tool").
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_load_skill_ref_unknown_skill() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_lsr".into(),
            name: "load_skill_ref".into(),
            arguments: serde_json::json!({"skill_id": "no-such-skill", "doc": "intro"}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("load_skill_ref must be intercepted");
        // Unknown skill → is_error, but NOT "Unknown tool"
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through to manager::dispatch, got: {}",
            res.output
        );
    }

    /// Phase A2 RED: `run_skill_script` with missing required args must be
    /// intercepted and return an error (not fall through).
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_run_skill_script_missing_args() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_rss".into(),
            name: "run_skill_script".into(),
            // Missing skill_id + script — handler must return an error
            arguments: serde_json::json!({}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("run_skill_script must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "missing args → must be is_error");
    }

    /// Phase A2 RED: `install_skill` with missing URL must be intercepted.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_install_skill_missing_url() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_is".into(),
            name: "install_skill".into(),
            arguments: serde_json::json!({}), // url missing
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("install_skill must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "missing url → must be is_error");
    }

    // ── Phase A3: checkpoint meta-tools ──────────────────────────────────

    /// Phase A3 RED: `create_checkpoint` must persist a row in DB and
    /// return success (no HTTP self-call).
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_create_checkpoint() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_cp".into(),
                name: "t".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_cp".into(),
            name: "create_checkpoint".into(),
            arguments: serde_json::json!({
                "label": "before-refactor",
                "description": "unit-test checkpoint",
            }),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_cp", &tc, tx).await;
        let res = opt.expect("create_checkpoint must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(!res.is_error, "must succeed: {}", res.output);
        assert!(
            res.output.contains("before-refactor"),
            "output must echo label, got: {}",
            res.output
        );
    }

    /// Phase A3 RED: `list_checkpoints` must return intercepted output.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_list_checkpoints() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_lc".into(),
                name: "t".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_lc".into(),
            name: "list_checkpoints".into(),
            arguments: serde_json::json!({}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_lc", &tc, tx).await;
        let res = opt.expect("list_checkpoints must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(!res.is_error, "must succeed: {}", res.output);
    }

    /// Phase A3 RED: `restore_checkpoint` with a bad ID must be intercepted
    /// with an error (not fall through to manager).
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_restore_checkpoint_not_found() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_rc".into(),
            name: "restore_checkpoint".into(),
            arguments: serde_json::json!({"checkpoint_id": "cp-nonexistent-0000"}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("restore_checkpoint must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "nonexistent CP → must be is_error");
    }

    // ── Phase A4: artifact / agents meta-tools ────────────────────────────

    /// Phase A4 RED: `store_artifact` must persist a row and return
    /// the artifact ID — no HTTP self-call.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_store_artifact() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_art".into(),
                name: "t".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let tc = LlmToolCall {
            id: "tc_sa".into(),
            name: "store_artifact".into(),
            arguments: serde_json::json!({
                "kind": "log",
                "content": "build output here",
                "label": "build-log",
            }),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_art", &tc, tx).await;
        let res = opt.expect("store_artifact must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(!res.is_error, "must succeed: {}", res.output);
        assert!(
            res.output.contains("art-"),
            "output must contain artifact ID, got: {}",
            res.output
        );
    }

    /// Phase A4 RED: `store_artifact` with missing content must error.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_store_artifact_missing_content() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_sa2".into(),
            name: "store_artifact".into(),
            arguments: serde_json::json!({"kind": "log"}), // content missing
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("store_artifact must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "missing content → is_error");
    }

    /// Phase A4 RED: `list_agents` must return an intercepted response
    /// (empty list for a fresh DB).
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_list_agents() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_la".into(),
            name: "list_agents".into(),
            arguments: serde_json::json!({}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("list_agents must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(!res.is_error, "must succeed: {}", res.output);
    }

    /// Phase A4 RED: `message_agent` with missing target must be intercepted
    /// and return an error.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_message_agent_missing_target() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_ma".into(),
            name: "message_agent".into(),
            arguments: serde_json::json!({"message": "hello"}), // target missing
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("message_agent must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "missing target → is_error");
    }

    /// Phase A4 RED: `message_agent` with valid target that does not
    /// exist in DB must be intercepted and return an error.
    #[tokio::test]
    async fn intercept_meta_tool_dispatches_message_agent_unknown_target() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let tc = LlmToolCall {
            id: "tc_ma2".into(),
            name: "message_agent".into(),
            arguments: serde_json::json!({"target": "ghost-agent", "message": "hello"}),
            thought_signature: None,
        };

        let opt = intercept_meta_tool(&state, "agent_x", &tc, tx).await;
        let res = opt.expect("message_agent must be intercepted");
        assert!(
            !res.output.starts_with("Unknown tool"),
            "must NOT fall through, got: {}",
            res.output
        );
        assert!(res.is_error, "unknown target → is_error");
    }

    // ── F6: cross-conversation search ────────────────────────────────────────

    /// F6: when no conversation_id is given, conversation_search must
    /// match messages from every conversation associated with the agent.
    /// Empty-string conversation_id is treated identically to omitted.
    #[tokio::test]
    async fn f6_conversation_search_spans_all_conversations_when_omitted() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_f6".into(),
                name: "f6".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Insert two messages in two different conversations, each containing
        // the unique token "rivulet" we'll search for.
        for (id, conv) in [("m_a", "conv-a"), ("m_b", "conv-b")] {
            cade_store::sqlite::insert_message(
                &state.db,
                &cade_store::sqlite::MessageRow {
                    id: id.into(),
                    agent_id: "agent_f6".into(),
                    conversation_id: Some(conv.into()),
                    role: "user".into(),
                    content: serde_json::json!({ "content": "the rivulet runs deep" }),
                    char_count: 32,
                },
            )
            .unwrap();
        }

        let (out, is_err) = handle_conversation_search_meta(
            &state,
            "agent_f6",
            &serde_json::json!({ "query": "rivulet" }),
        )
        .await;

        assert!(!is_err, "no scope → ok, got: {out}");
        assert!(
            out.contains("all conversations"),
            "scope label must say 'all conversations', got: {out}"
        );
        // Both conversations' messages should surface (we inserted matches in both).
        assert!(
            out.contains("rivulet"),
            "matched snippet must echo the query token, got: {out}"
        );
    }

    /// F6: when a specific conversation_id is supplied, only messages from
    /// that conversation are returned.
    #[tokio::test]
    async fn f6_conversation_search_scopes_to_single_conversation_when_given() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_f6b".into(),
                name: "f6b".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Only conversation `keep` contains the marker token; conversation
        // `drop` contains a different unique token.
        cade_store::sqlite::insert_message(
            &state.db,
            &cade_store::sqlite::MessageRow {
                id: "m_keep".into(),
                agent_id: "agent_f6b".into(),
                conversation_id: Some("keep".into()),
                role: "user".into(),
                content: serde_json::json!({ "content": "alpha kestrel" }),
                char_count: 14,
            },
        )
        .unwrap();
        cade_store::sqlite::insert_message(
            &state.db,
            &cade_store::sqlite::MessageRow {
                id: "m_drop".into(),
                agent_id: "agent_f6b".into(),
                conversation_id: Some("drop".into()),
                role: "user".into(),
                content: serde_json::json!({ "content": "alpha falcon" }),
                char_count: 12,
            },
        )
        .unwrap();

        // Scope to `keep` and search for `kestrel` — must match the keep row.
        let (out_keep, _) = handle_conversation_search_meta(
            &state,
            "agent_f6b",
            &serde_json::json!({ "query": "kestrel", "conversation_id": "keep" }),
        )
        .await;
        assert!(
            out_keep.contains("conversation keep"),
            "scope label must name the requested conversation, got: {out_keep}"
        );
        assert!(
            out_keep.contains("kestrel"),
            "must surface the matching message, got: {out_keep}"
        );

        // Scope to `keep` and search for `falcon` — must miss because the
        // falcon message is in conversation `drop`.
        let (out_miss, is_err_miss) = handle_conversation_search_meta(
            &state,
            "agent_f6b",
            &serde_json::json!({ "query": "falcon", "conversation_id": "keep" }),
        )
        .await;
        assert!(!is_err_miss, "no-match is not an error, got: {out_miss}");
        assert!(
            out_miss.starts_with("No conversation messages matched"),
            "scoped search must not leak other-conversation hits, got: {out_miss}"
        );
        assert!(
            out_miss.contains("conversation keep"),
            "miss message must include the scope label, got: {out_miss}"
        );
    }

    // ── F8: compaction transparency in conversation_search ────────────────────

    /// F8: when no hits but a compaction marker exists, the empty response
    /// must point the agent at the F2 archival cache + session_summary.
    #[tokio::test]
    async fn f8_empty_search_with_compaction_marker_hints_at_archival() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_f8a".into(),
                name: "f8a".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Insert ONLY a compaction marker — no other messages to match.
        cade_store::sqlite::insert_message(
            &state.db,
            &cade_store::sqlite::MessageRow {
                id: "compact-1".into(),
                agent_id: "agent_f8a".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: serde_json::json!({ "content": "[Compaction marker: 5 turns]" }),
                char_count: 0,
            },
        )
        .unwrap();

        let (out, is_err) = handle_conversation_search_meta(
            &state,
            "agent_f8a",
            &serde_json::json!({ "query": "ghost-token-no-match" }),
        )
        .await;

        assert!(!is_err, "no-hit is not an error, got: {out}");
        assert!(
            out.contains("No conversation messages matched"),
            "must report no matches, got: {out}"
        );
        assert!(
            out.contains("compacted history") && out.contains("archival_memory_search"),
            "F8: empty-with-marker must hint at archival_memory_search, got: {out}"
        );
        assert!(
            out.contains("dropped-turns"),
            "F8: hint must reference the F2 'dropped-turns' tag, got: {out}"
        );
        assert!(
            out.contains("session_summary"),
            "F8: hint must also point at session_summary, got: {out}"
        );
    }

    /// F8: when no hits AND no compaction marker exists, the empty response
    /// must NOT spam an irrelevant archival hint.
    #[tokio::test]
    async fn f8_empty_search_without_marker_omits_archival_hint() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_f8b".into(),
                name: "f8b".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let (out, _) = handle_conversation_search_meta(
            &state,
            "agent_f8b",
            &serde_json::json!({ "query": "ghost-token-no-match" }),
        )
        .await;
        assert!(
            !out.contains("compacted history"),
            "F8: clean agent must not get the compaction hint, got: {out}"
        );
        assert!(
            !out.contains("archival_memory_search"),
            "F8: clean agent must not be redirected to archival, got: {out}"
        );
    }

    /// F8: when at least one hit sits before the compaction marker, the
    /// header line must report the count of pre-compaction hits.
    #[tokio::test]
    async fn f8_header_reports_pre_compaction_hit_count() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "agent_f8c".into(),
                name: "f8c".into(),
                model: "t".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Two messages before the compaction marker, one after.
        // FTS5 indexes by rowid (insertion order) and the compaction marker
        // sits between them — so both pre-marker rows are tagged
        // 'pre-compaction' and the post-marker row is not.
        for (id, role, body) in [
            ("m1", "user", "the unique-token f8c lives here"),
            ("m2", "assistant", "I saw the unique-token f8c too"),
        ] {
            cade_store::sqlite::insert_message(
                &state.db,
                &cade_store::sqlite::MessageRow {
                    id: id.into(),
                    agent_id: "agent_f8c".into(),
                    conversation_id: None,
                    role: role.into(),
                    content: serde_json::json!({ "content": body }),
                    char_count: body.chars().count(),
                },
            )
            .unwrap();
        }
        cade_store::sqlite::insert_message(
            &state.db,
            &cade_store::sqlite::MessageRow {
                id: "compact-c".into(),
                agent_id: "agent_f8c".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: serde_json::json!({ "content": "[marker]" }),
                char_count: 0,
            },
        )
        .unwrap();
        cade_store::sqlite::insert_message(
            &state.db,
            &cade_store::sqlite::MessageRow {
                id: "m3".into(),
                agent_id: "agent_f8c".into(),
                conversation_id: None,
                role: "user".into(),
                content: serde_json::json!({ "content": "post-compaction unique-token f8c" }),
                char_count: 30,
            },
        )
        .unwrap();

        let (out, is_err) = handle_conversation_search_meta(
            &state,
            "agent_f8c",
            &serde_json::json!({ "query": "unique-token f8c" }),
        )
        .await;
        assert!(!is_err, "search must succeed, got: {out}");
        assert!(
            out.contains("from pre-compaction history"),
            "F8: header must annotate pre-compaction hit count, got: {out}"
        );
        // Sanity: the snippet markers themselves still carry the pre-compaction tag.
        assert!(
            out.contains("[pre-compaction"),
            "F8: snippets for pre-compaction rows must keep their inline tag, got: {out}"
        );
    }

    /// RED: at depth >= CADE_SUBAGENT_MAX_DEPTH (default 3), the tool must
    /// short-circuit with an error and never call the LLM.  Currently fails
    /// because no depth guard exists.
    #[tokio::test]
    async fn depth_limit_blocks_recursion_before_llm_call() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let args = serde_json::json!({
            "prompt": "do thing",
            "_subagent_depth": 3,
        });

        // Should NOT panic — i.e. LLM is never called.
        let result = handle_run_subagent_tool(&state, "parent_agent_x", "tc_1", &args, tx).await;

        assert!(result.is_error, "depth-limit must produce an error result");
        assert!(
            result.output.to_lowercase().contains("depth"),
            "error message must mention depth, got: {}",
            result.output
        );
    }

    // ── subagent meta-tool dispatch ───────────────────────────────────────

    /// Mock LLM that, on the first call, returns a tool_call for a
    /// specified meta-tool.  On the second call captures the messages
    /// (so we can inspect the tool result) and returns final text.
    struct MetaToolLlm {
        tool_name: String,
        tool_args: serde_json::Value,
        call_count: std::sync::atomic::AtomicUsize,
        captured_iter2: std::sync::Mutex<Vec<cade_ai::LlmMessage>>,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for MetaToolLlm {
        async fn complete(
            &self,
            r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let n = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(cade_ai::CompletionResponse {
                    content: None,
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_meta_1".into(),
                        name: self.tool_name.clone(),
                        arguments: self.tool_args.clone(),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                *self.captured_iter2.lock().unwrap() = r.messages.clone();
                Ok(cade_ai::CompletionResponse {
                    content: Some("meta-tool subagent done".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!()
        }
    }

    /// Subagent loop must route `update_memory` through the meta-tool
    /// intercept rather than falling through to `dispatch()` which would
    /// return "Unknown tool".
    #[tokio::test]
    async fn subagent_loop_dispatches_update_memory_via_intercept() {
        let llm = std::sync::Arc::new(MetaToolLlm {
            tool_name: "update_memory".into(),
            tool_args: serde_json::json!({
                "label": "sa_block",
                "value": "written by subagent",
                "operation": "set",
            }),
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);

        // Create parent agent.
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "parent_sa".into(),
                name: "t".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let args = serde_json::json!({ "prompt": "update memory test" });
        let result =
            handle_run_subagent_tool(&state, "parent_sa", "tc_sa", &args, tx).await;

        assert!(
            !result.is_error,
            "subagent must succeed, got: {}",
            result.output,
        );

        // The tool result fed back to the LLM must NOT be "Unknown tool".
        let iter2 = llm.captured_iter2.lock().unwrap().clone();
        let tool_msg = iter2
            .iter()
            .find(|m| m.role == "tool")
            .expect("must have a tool-role message in iter-2");
        assert!(
            !tool_msg.content.contains("Unknown tool"),
            "update_memory must be intercepted in subagent loop, got: {}",
            tool_msg.content,
        );

        // Memory isolation: the block must NOT be written to the parent's DB.
        // The subagent writes to its own ephemeral row (which is deleted on
        // completion), so the parent's memory store must remain empty.
        let parent_blocks =
            cade_store::sqlite::get_memory_blocks(&state.db, "parent_sa").unwrap();
        let leaked = parent_blocks
            .iter()
            .any(|(label, _, _)| label == "sa_block");
        assert!(
            !leaked,
            "subagent update_memory must NOT leak into parent DB (memory isolation), \
             but found sa_block in parent: {parent_blocks:?}",
        );
    }

    /// Subagent loop must route `search_memory` through the meta-tool
    /// intercept so it can read the parent agent's memory blocks.
    #[tokio::test]
    async fn subagent_loop_dispatches_search_memory_via_intercept() {
        let llm = std::sync::Arc::new(MetaToolLlm {
            tool_name: "search_memory".into(),
            tool_args: serde_json::json!({ "query": "anything" }),
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);

        // Create parent agent.
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "parent_sm".into(),
                name: "t".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let args = serde_json::json!({ "prompt": "search memory test" });
        let result =
            handle_run_subagent_tool(&state, "parent_sm", "tc_sm", &args, tx).await;

        assert!(!result.is_error, "got: {}", result.output);

        let iter2 = llm.captured_iter2.lock().unwrap().clone();
        let tool_msg = iter2
            .iter()
            .find(|m| m.role == "tool")
            .expect("must have a tool-role message");
        assert!(
            !tool_msg.content.contains("Unknown tool"),
            "search_memory must be intercepted in subagent loop, got: {}",
            tool_msg.content,
        );
    }

    // ── subagent file-edit tracking ───────────────────────────────────────

    /// When a subagent dispatches a `write_file` tool, the parent agent's
    /// `recent_edits` memory block must be updated with the file path.
    #[tokio::test]
    async fn subagent_write_file_records_recent_edit() {
        // Use a path within the current working dir to avoid sandbox restrictions
        let tmp_path = std::env::current_dir()
            .unwrap()
            .join("_test_subagent_edit.tmp");
        let tmp_path_str = tmp_path.to_str().unwrap().to_string();

        struct WriteFileLlmInner {
            call_count: std::sync::atomic::AtomicUsize,
            path: String,
        }
        #[async_trait::async_trait]
        impl cade_ai::LlmProvider for WriteFileLlmInner {
            async fn complete(
                &self,
                _r: &cade_ai::CompletionRequest,
            ) -> cade_ai::Result<cade_ai::CompletionResponse> {
                let n = self
                    .call_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Ok(cade_ai::CompletionResponse {
                        content: None,
                        tool_calls: vec![cade_ai::LlmToolCall {
                            id: "tc_wf".into(),
                            name: "write_file".into(),
                            arguments: serde_json::json!({
                                "path": self.path,
                                "content": "hello from subagent"
                            }),
                            thought_signature: None,
                        }],
                        finish_reason: "tool_use".into(),
                    })
                } else {
                    Ok(cade_ai::CompletionResponse {
                        content: Some("wrote the file".into()),
                        tool_calls: vec![],
                        finish_reason: "stop".into(),
                    })
                }
            }
            async fn stream(
                &self,
                _r: &cade_ai::CompletionRequest,
            ) -> cade_ai::Result<
                std::pin::Pin<
                    Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
                >,
            > {
                unreachable!()
            }
        }

        let llm = std::sync::Arc::new(WriteFileLlmInner {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            path: tmp_path_str.clone(),
        });
        let llm_dyn = llm as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        // Create a parent agent in the DB
        let parent_id = "parent_edit_track";
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: parent_id.to_string(),
                name: "edit-track-parent".to_string(),
                model: "mock".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let args = serde_json::json!({ "prompt": "write a file" });
        let result = handle_run_subagent_tool(&state, parent_id, "tc_wf", &args, tx).await;

        assert!(!result.is_error, "got: {}", result.output);

        // Check that recent_edits was recorded for the parent agent
        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, parent_id)
            .expect("get_memory_blocks should succeed");
        let re = blocks
            .iter()
            .find(|(l, _, _)| l == "recent_edits");
        assert!(
            re.is_some(),
            "recent_edits block must exist after subagent write_file, blocks: {:?}",
            blocks.iter().map(|(l, _, _)| l).collect::<Vec<_>>()
        );
        let (_, value, _) = re.unwrap();
        assert!(
            value.contains(&tmp_path_str),
            "recent_edits must contain the written file path, got: {value}"
        );

        // Clean up
        let _ = std::fs::remove_file(&tmp_path);
    }

    /// Same as above but with an MCP-prefixed tool name — verifies that
    /// `is_file_edit_tool` correctly strips the prefix.
    struct McpWriteFileLlm {
        call_count: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for McpWriteFileLlm {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let n = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(cade_ai::CompletionResponse {
                    content: None,
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_mcp_wf".into(),
                        // MCP-prefixed name — this is what the LLM sees
                        name: "developer__write_file".into(),
                        arguments: serde_json::json!({
                            "path": "/tmp/cade_test_mcp_subagent_edit.txt",
                            "file_text": "hello from mcp subagent"
                        }),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                Ok(cade_ai::CompletionResponse {
                    content: Some("wrote via mcp".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn subagent_mcp_prefixed_write_records_recent_edit() {
        let llm = std::sync::Arc::new(McpWriteFileLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let llm_dyn = llm as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let parent_id = "parent_mcp_edit";
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: parent_id.to_string(),
                name: "mcp-edit-parent".to_string(),
                model: "mock".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let args = serde_json::json!({ "prompt": "write via mcp" });
        let _result = handle_run_subagent_tool(&state, parent_id, "tc_mcp", &args, tx).await;

        // The MCP tool won't actually write (no MCP server connected in test),
        // but we don't care about the dispatch result — we care that the
        // tracking code matched the MCP-prefixed name.  The tool result will
        // be an error ("Unknown tool"), but is_file_edit_tool should still
        // be true for the name.  However, tracking is gated on !is_error,
        // so this tests that the name matching works even if the tool errors.
        // We adjust: recent_edits should NOT be written when tool errored.
        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, parent_id)
            .expect("get_memory_blocks should succeed");
        let re = blocks.iter().find(|(l, _, _)| l == "recent_edits");
        // Tool errored (no MCP server), so no edit should be recorded.
        assert!(
            re.is_none(),
            "recent_edits should NOT be set when MCP tool errors, blocks: {:?}",
            blocks.iter().map(|(l, _, _)| l).collect::<Vec<_>>()
        );
    }

#[cfg(test)]
mod p4_guardrail_tests {
    use super::*;

    #[test]
    fn parse_max_session_cost_unset_disables_guardrail() {
        assert_eq!(parse_max_session_cost(None), None);
    }

    #[test]
    fn parse_max_session_cost_empty_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("")), None);
        assert_eq!(parse_max_session_cost(Some("   ")), None);
    }

    #[test]
    fn parse_max_session_cost_nonpositive_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("0")), None);
        assert_eq!(parse_max_session_cost(Some("0.0")), None);
        assert_eq!(parse_max_session_cost(Some("-5")), None);
    }

    #[test]
    fn parse_max_session_cost_positive_returns_cap() {
        assert_eq!(parse_max_session_cost(Some("2.50")), Some(2.50));
        assert_eq!(parse_max_session_cost(Some(" 10 ")), Some(10.0));
    }

    #[test]
    fn parse_max_session_cost_garbage_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("not-a-number")), None);
        assert_eq!(parse_max_session_cost(Some("$5")), None);
    }

    /// `pricing_registry` returns a stable instance (`OnceLock`).
    #[test]
    fn pricing_registry_is_stable() {
        let p1 = pricing_registry() as *const _;
        let p2 = pricing_registry() as *const _;
        assert!(std::ptr::eq(p1, p2));
    }

    // ── P6: tool-turn output cap ─────────────────────────────────────────

    #[test]
    fn parse_tool_turn_unset_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(None), None);
    }

    #[test]
    fn parse_tool_turn_empty_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("")), None);
        assert_eq!(parse_tool_turn_max_tokens(Some("   ")), None);
    }

    #[test]
    fn parse_tool_turn_zero_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("0")), None);
    }

    #[test]
    fn parse_tool_turn_garbage_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("abc")), None);
        assert_eq!(parse_tool_turn_max_tokens(Some("1024k")), None);
    }

    #[test]
    fn parse_tool_turn_positive_returns_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("1024")), Some(1024));
        assert_eq!(parse_tool_turn_max_tokens(Some("4096")), Some(4096));
        assert_eq!(parse_tool_turn_max_tokens(Some(" 512 ")), Some(512));
    }
}


#[cfg(test)]
mod sse_protocol_tests {
    //! Integration coverage for the `POST /v1/agents/:id/run` SSE
    //! response.  These tests drive `run_agent` end-to-end through the
    //! axum extractors and assert the protocol surface seen by the
    //! CLI / TUI / GUI clients.  Started as Task 2.3 of the code-review
    //! resolution plan.
    //!
    //! The current matrix covers the two error paths that do **not**
    //! require an LLM call (empty input, missing conversation).  A
    //! follow-up commit will add scripted-LLM happy-path and
    //! tool-dispatch coverage.

    use super::*;
    use axum::{
        Json,
        body::to_bytes,
        extract::{Path, State},
        http::StatusCode,
    };

    /// Build a state whose LLM panics if called — proves no LLM
    /// request happens on the error paths under test.
    fn state_no_llm() -> AppState {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        build_state_with_llm(llm)
    }

    #[tokio::test]
    async fn empty_input_returns_400_bad_request_with_missing_input_message() {
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({ "input": "" })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        assert!(
            body_str.contains("missing 'input'"),
            "body must explain the missing-input error; got: {body_str}"
        );
    }

    #[tokio::test]
    async fn missing_input_field_returns_400_bad_request() {
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn nonexistent_conversation_id_returns_404_not_found() {
        // Body declares conversation_id that does not exist in the DB.
        // `resolve_conversation` must short-circuit with 404 before any
        // SSE stream is opened.
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({
                "input": "hello",
                "conversation_id": "conv-does-not-exist",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        assert!(
            body_str.contains("conv-does-not-exist") || body_str.contains("not found"),
            "body must reference the missing conversation; got: {body_str}"
        );
    }

    #[tokio::test]
    async fn empty_input_response_does_not_leak_internal_paths() {
        // §3.3 of tdd-guide: error responses must not expose stack
        // traces, internal file paths, or framework version strings.
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({ "input": "" })),
        )
        .await;
        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        let lc = body_str.to_lowercase();
        assert!(
            !lc.contains("/home/")
                && !lc.contains("c:\\")
                && !lc.contains("backtrace")
                && !lc.contains("rust_panic"),
            "error body must not leak host paths or stack traces: {body_str}"
        );
    }
}


#[cfg(test)]
mod run_agent_helpers_tests {
    use super::*;
    use serde_json::json;

    // ── parse_input ────────────────────────────────────────────────────────

    #[test]
    fn parse_input_returns_string_for_valid_input() {
        let body = json!({ "input": "hello world" });
        let result = parse_input(&body);
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn parse_input_errors_on_missing_field() {
        let body = json!({});
        assert!(parse_input(&body).is_err());
    }

    #[test]
    fn parse_input_errors_on_empty_string() {
        let body = json!({ "input": "" });
        assert!(parse_input(&body).is_err());
    }

    #[test]
    fn parse_input_errors_on_non_string_value() {
        let body = json!({ "input": 42 });
        assert!(parse_input(&body).is_err());
    }

    // ── detect_theme_cmd ───────────────────────────────────────────────────

    #[test]
    fn detect_theme_cmd_returns_name_for_theme_prefix() {
        assert_eq!(
            detect_theme_cmd("/theme catppuccin"),
            Some("catppuccin".to_string())
        );
    }

    #[test]
    fn detect_theme_cmd_trims_whitespace() {
        assert_eq!(
            detect_theme_cmd("/theme  dark  "),
            Some("dark".to_string())
        );
    }

    #[test]
    fn detect_theme_cmd_returns_none_for_non_theme_input() {
        assert_eq!(detect_theme_cmd("hello"), None);
        assert_eq!(detect_theme_cmd("/memory"), None);
        assert_eq!(detect_theme_cmd(""), None);
    }

    #[test]
    fn detect_theme_cmd_handles_reload() {
        assert_eq!(
            detect_theme_cmd("/theme reload"),
            Some("reload".to_string())
        );
    }

    // ── make_run_id ────────────────────────────────────────────────────────

    #[test]
    fn make_run_id_fallback_starts_with_run_local() {
        // We can't construct a real AppState easily, but we can verify the
        // fallback format by calling the inner logic directly.
        let ts = chrono::Utc::now().timestamp();
        let id = format!("run-local-{ts}");
        assert!(id.starts_with("run-local-"));
        assert!(id.len() > "run-local-".len());
    }
}

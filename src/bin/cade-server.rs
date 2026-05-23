// region:    --- Modules

use axum::http::{HeaderValue, Method, Request};
use cade::{Error, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use cade::server::{
    api::router,
    config::ServerConfig,
    rate_limit::RateLimiter,
    state::{AppState, McpManager},
};
use cade_store::sqlite::{self, open as open_db};

use cade::settings::SettingsManager;
use cade_ai::{CompletionRequest, LlmProvider, LlmRouter};

// endregion: --- Modules

/// CADE server — LLM gateway and agent state store
#[derive(Parser, Debug)]
#[command(name = "cade-server", version, about)]
struct ServerArgs {
    /// Port to listen on (overrides CADE_SERVER_PORT env var, default 8284)
    #[arg(
        long = "port",
        short = 'p',
        env = "CADE_SERVER_PORT",
        default_value_t = 8284
    )]
    port: u16,
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024) // 16 MB — prevents stack overflow from deeply-nested async state machines
        // (run_agent_loop → build_context → consolidate_agent chain)
        .build()
        .map_err(|e| Error::custom(format!("tokio runtime: {e}")))?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Install a panic hook that logs the panic before the process unwinds.
    // Without this, panics on background tokio tasks may be invisible.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("PANIC: {info}");
        original_hook(info);
    }));

    let _ = dotenvy::dotenv();

    // Parse CLI args first so --port / CADE_SERVER_PORT is available
    let args = ServerArgs::parse();

    let config = ServerConfig::from_env_with_port(Some(args.port))
        .map_err(|e| Error::custom(e.to_string()))?;

    tracing::info!(
        "CADE Server v{} | provider={} | db={}",
        env!("CARGO_PKG_VERSION"),
        config.llm_provider,
        config.db_path
    );

    let db = open_db(&config.db_path)
        .map_err(|e: cade_store::error::Error| Error::custom(e.to_string()))?;

    // Build router from env vars first
    let ai_config = config.to_ai_config();
    let mut router_inner = LlmRouter::build(&ai_config);

    // Hot-load any providers persisted in the DB (DB overrides env vars)
    let db_providers = match sqlite::list_providers(&db) {
        Ok(providers) => providers,
        Err(e) => {
            tracing::warn!(
                "Could not load providers from DB: {e}. Continuing with env-var providers only."
            );
            vec![]
        }
    };
    for row in &db_providers {
        if !row.enabled {
            continue;
        }
        if let Some(p) = LlmRouter::provider_from_row(
            &row.kind,
            row.api_key.clone(),
            row.base_url.clone(),
            &ai_config,
        ) {
            // Store the API key so list_dynamic_models() can fetch live model lists.
            let key = row.api_key.clone().unwrap_or_default();
            router_inner.add_provider_with_key(row.name.clone(), p, key);
            tracing::info!("Loaded provider from DB: {} ({})", row.name, row.kind);
        }
    }

    tracing::info!(
        "Active providers: {}",
        router_inner.provider_names().join(", ")
    );

    let llm_router = Arc::new(RwLock::new(router_inner));
    // llm field: thin Arc pointing to the router itself (router implements LlmProvider)
    let llm: Arc<dyn LlmProvider> = {
        // We need a stable Arc<dyn LlmProvider> for the existing llm field.
        // Since LlmRouter implements LlmProvider via the RwLock wrapper,
        // we wrap the RwLock<LlmRouter> in a thin adapter.
        Arc::new(RouterAdapter(Arc::clone(&llm_router)))
    };

    // ── MCP servers ───────────────────────────────────────────────────────────
    // Load MCP server configs from the user's settings files (global + project)
    // and start each as a child process. The McpManager is shared across all
    // agentic-loop requests so connections are reused between turns.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mcp: Arc<McpManager> = match SettingsManager::new(&cwd) {
        Ok(settings) => {
            let mcp_configs = settings.merged_mcp_servers();
            if mcp_configs.is_empty() {
                tracing::info!(
                    "No MCP servers configured — agentic loop will use native tools only"
                );
                Arc::new(McpManager::empty())
            } else {
                tracing::info!("Starting {} MCP server(s)…", mcp_configs.len());
                let (mgr, _results) = McpManager::start(&mcp_configs).await;
                Arc::new(mgr)
            }
        }
        Err(e) => {
            tracing::warn!("Could not load settings for MCP: {e} — continuing without MCP servers");
            Arc::new(McpManager::empty())
        }
    };

    // ── Discover skills at boot ────────────────────────────────────────────
    let cwd = std::env::current_dir().unwrap_or_default();
    let all_skills = cade_core::skills::discover_all_skills(&cwd, None, None);
    tracing::info!("Discovered {} skills", all_skills.len());

    // ── Pre-register Tools (Native, Meta, MCP) ──────────────────────────────
    // The server needs these schemas inserted into SQLite at boot so that
    // the LLM context builder can see them without relying on the CLI.
    {
        use cade_agent::agent::tools::build_python_stub_from_schema;
        use cade_agent::tools::catalog::{
            meta_schemas_for_capabilities, native_schemas_for_capabilities,
        };
        use cade_store::sqlite::ToolRow;

        let caps = cade_core::capabilities::CapabilitySet::full();
        let meta_schemas = meta_schemas_for_capabilities(&caps);
        let native_schemas =
            native_schemas_for_capabilities(cade_core::toolsets::Toolset::Default, &caps);

        let mut total_registered = 0;

        for schema in meta_schemas {
            let name = schema["name"].as_str().unwrap_or("").to_string();
            let description = schema["description"].as_str().map(String::from);
            let row = ToolRow {
                id: format!("tool-{}", uuid::Uuid::new_v4()),
                name: name.clone(),
                description,
                source_code: Some(String::new()),
                json_schema: Some(schema),
                tags: vec!["cade".to_string(), "meta".to_string()],
            };
            if let Err(e) = cade_store::sqlite::upsert_tool(&db, &row) {
                tracing::warn!("Failed to pre-register meta tool {}: {}", name, e);
            } else {
                total_registered += 1;
            }
        }

        for schema in native_schemas {
            let name = schema["name"].as_str().unwrap_or("").to_string();
            let description = schema["description"].as_str().unwrap_or("").to_string();
            let stub = build_python_stub_from_schema(&name, &description, &schema["parameters"]);
            let row = ToolRow {
                id: format!("tool-{}", uuid::Uuid::new_v4()),
                name: name.clone(),
                description: Some(description),
                source_code: Some(stub),
                json_schema: Some(schema),
                tags: vec!["cade".to_string()],
            };
            if let Err(e) = cade_store::sqlite::upsert_tool(&db, &row) {
                tracing::warn!("Failed to pre-register native tool {}: {}", name, e);
            } else {
                total_registered += 1;
            }
        }

        let mcp_schemas = mcp.all_tool_schemas().await;
        for mut schema in mcp_schemas {
            let name = schema["name"].as_str().unwrap_or("").to_string();
            let description = schema["description"].as_str().map(String::from);
            let is_core = schema["_is_core"].as_bool().unwrap_or(false);
            if let Some(obj) = schema.as_object_mut() {
                obj.remove("_is_core");
            }

            let mut tags = vec!["cade".to_string(), "mcp".to_string()];
            if is_core {
                tags.push("core_mcp".to_string());
            }

            let stub = build_python_stub_from_schema(
                &name,
                description.as_deref().unwrap_or(""),
                &schema["parameters"],
            );
            let row = ToolRow {
                id: format!("tool-{}", uuid::Uuid::new_v4()),
                name: name.clone(),
                description,
                source_code: Some(stub),
                json_schema: Some(schema),
                tags,
            };
            if let Err(e) = cade_store::sqlite::upsert_tool(&db, &row) {
                tracing::warn!("Failed to pre-register MCP tool {}: {}", name, e);
            } else {
                total_registered += 1;
            }
        }

        tracing::info!(
            "Pre-registered {} total tools into the database at startup",
            total_registered
        );
    }

    let state = AppState {
        db,
        llm,
        llm_router,
        config: Arc::new(config.clone()),
        mcp,
        rate_limiter: RateLimiter::from_env(),
        memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(dashmap::DashMap::new()),
        agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
        context_cache: Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            cade_server::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: Arc::new(RwLock::new(all_skills)),
        agent_skills: Arc::new(RwLock::new(std::collections::HashMap::new())),
        pending_subagent_results: Arc::new(RwLock::new(std::collections::HashMap::new())),
        subagent_cancellations: Arc::new(RwLock::new(std::collections::HashMap::new())),
        subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(
            std::env::var("CADE_MAX_SUBAGENTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
        )),
        // ── Embedder (WI-SEMANTIC Phase 4) ───────────────────────────────────
        // Built only with `--features semantic-search`. First call downloads
        // ~25 MB of MiniLM-L6-v2-Q weights into the user cache dir; subsequent
        // calls reuse them. Failures fall back to None so the server still
        // boots and `search_memory_hybrid` transparently degrades to the
        // keyword-only path.
        embedder: tokio::task::spawn_blocking(|| {
            #[cfg(feature = "semantic-search")]
            {
                match cade_store::sqlite::embedding::FastEmbedder::new() {
                    Ok(e) => {
                        tracing::info!("Semantic search embedder initialised (FastEmbedder)");
                        Some(Arc::new(e) as Arc<dyn cade_store::sqlite::embedding::Embedder>)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Semantic search requested but FastEmbedder init failed: {e}; \
                             falling back to keyword-only memory search"
                        );
                        None
                    }
                }
            }
            #[cfg(not(feature = "semantic-search"))]
            {
                None
            }
        })
        .await
        .unwrap_or(None),
    };

    // ── Embedding backfill (WI-SEMANTIC Phase 4) ────────────────────────────
    // When the `semantic-search` feature is active and the embedder
    // initialised successfully, fill in embeddings for any pre-existing
    // memory blocks (rows where `embedding IS NULL`). Runs once at startup
    // in a blocking thread so it doesn't stall the async runtime.
    if let Some(emb) = state.embedder.clone() {
        let db_bf = state.db.clone();
        tokio::task::spawn_blocking(
            move || match cade_store::sqlite::embedding::backfill_embeddings(&db_bf, &*emb) {
                Ok(n) if n > 0 => {
                    tracing::info!("Embedding backfill: filled {n} memory block(s) at startup")
                }
                Ok(_) => tracing::debug!("Embedding backfill: nothing to do"),
                Err(e) => tracing::warn!("Embedding backfill failed: {e}"),
            },
        );
    }

    // ── Sleeptime consolidation task ─────────────────────────────────────────
    // Polls every 30 s.  When an agent has been inactive for 20 s AND its
    // build_context dropped turns in the last request, call consolidate_agent
    // to summarise the dropped turns into the `session_summary` memory block.
    // M3: threshold lowered from 60 s → 20 s so interactive pauses trigger
    // consolidation sooner; turn-count eager path (see build_context) covers
    // continuous sessions that never hit the idle timer.
    let state_bg = state.clone();
    // RC7-FIX: Semaphore limits concurrent consolidation tasks to prevent
    // unbounded task spawning when many agents need consolidation at once.
    let consolidation_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            let mut pending: Vec<(String, Option<String>)> = Vec::new();
            {
                let mut activity = state_bg.agent_activity.write().await;
                let now = chrono::Utc::now().timestamp();
                for (agent_id, act) in activity.iter_mut() {
                    if act.needs_consolidation && (now - act.last_active_ts) > 20 {
                        act.needs_consolidation = false;
                        pending.push((agent_id.clone(), act.conversation_id.clone()));
                    }
                }
            }

            for (agent_id, conv_id) in pending {
                tracing::info!(
                    "Sleeptime consolidation triggered for agent {} (conv={:?})",
                    agent_id,
                    conv_id
                );
                let state_c = state_bg.clone();
                let sem = consolidation_semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    cade::server::consolidation::consolidate_agent(
                        &state_c,
                        &agent_id,
                        conv_id.as_deref(),
                        None,
                    )
                    .await;
                });
            }
        }
    });

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
            tracing::info_span!("http", method = %req.method(), uri = %req.uri())
        })
        .on_failure(
            tower_http::trace::DefaultOnFailure::new().level(tracing::Level::ERROR)
        );

    let mut allowed_origins = vec![
        "http://localhost"
            .parse::<HeaderValue>()
            .expect("valid header"),
        format!("http://localhost:{}", config.addr.port())
            .parse::<HeaderValue>()
            .expect("valid header"),
        "http://127.0.0.1"
            .parse::<HeaderValue>()
            .expect("valid header"),
        format!("http://127.0.0.1:{}", config.addr.port())
            .parse::<HeaderValue>()
            .expect("valid header"),
    ];

    if let Some(origin) = &config.allowed_origin
        && let Ok(parsed) = origin.parse::<HeaderValue>()
    {
        allowed_origins.push(parsed);
    }

    let app = router(state)
        .layer(axum::middleware::map_response(add_version_header))
        .layer(
            // H-03: Restrict CORS to localhost origins only (not permissive/open)
            CorsLayer::new()
                .allow_origin(allowed_origins)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(tower_http::cors::Any),
        )
        .layer(trace_layer);

    tracing::info!("Listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// -- Version header middleware

async fn add_version_header(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("x-cade-version"),
        axum::http::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
    response
}

// -- RouterAdapter: thin wrapper so Arc<RwLock<LlmRouter>> implements LlmProvider
//
// IMPORTANT: the lock is held ONLY for the brief resolve_provider() call.
// It is dropped BEFORE any HTTP calls to Anthropic / OpenAI / Gemini.

struct RouterAdapter(Arc<RwLock<LlmRouter>>);

#[async_trait::async_trait]
impl LlmProvider for RouterAdapter {
    async fn complete(
        &self,
        req: &CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        let (provider, bare_model) = {
            let router = self.0.read().await;
            router.resolve_provider(&req.model)?
        };
        let routed = CompletionRequest {
            model: bare_model,
            ..req.clone()
        };
        provider.complete(&routed).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        let (provider, bare_model) = {
            let router = self.0.read().await;
            router.resolve_provider(&req.model)?
        };
        let routed = CompletionRequest {
            model: bare_model,
            ..req.clone()
        };

        provider.stream(&routed).await
    }
}

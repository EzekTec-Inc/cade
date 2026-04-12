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
    state::AppState,
};
use cade_store::sqlite::{open as open_db, self};

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

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

    let db = open_db(&config.db_path).map_err(|e: cade_store::error::Error| Error::custom(e.to_string()))?;


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

    let state = AppState {
        db,
        llm,
        llm_router,
        config: Arc::new(config.clone()),
        rate_limiter: RateLimiter::from_env(),
        memory_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
    };

    // ── Sleeptime consolidation task ─────────────────────────────────────────
    // Polls every 30 s.  When an agent has been inactive for 60 s AND its
    // build_context dropped turns in the last request, call consolidate_agent
    // to summarise the dropped turns into the `session_summary` memory block.
    let state_bg = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            let mut pending: Vec<(String, Option<String>)> = Vec::new();
            {
                let mut activity = state_bg.agent_activity.write().await;
                let now = chrono::Utc::now().timestamp();
                for (agent_id, act) in activity.iter_mut() {
                    if act.needs_consolidation && (now - act.last_active_ts) > 60 {
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
                tokio::spawn(async move {
                    cade::server::consolidation::consolidate_agent(
                        &state_c,
                        &agent_id,
                        conv_id.as_deref(),
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

    let app = router(state)
        .layer(axum::middleware::map_response(add_version_header))
        .layer(
            // H-03: Restrict CORS to localhost origins only (not permissive/open)
            CorsLayer::new()
                .allow_origin([
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
                ])
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

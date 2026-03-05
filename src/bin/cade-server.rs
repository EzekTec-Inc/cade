use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use axum::http::{Request, HeaderValue, Method};

use cade::server::{
    api::router,
    config::ServerConfig,
    llm::{CompletionRequest, LlmRouter},
    rate_limit::{RateLimiter, rate_limit_middleware},
    state::AppState,
    storage::{open as open_db, sqlite},
};

/// CADE server — LLM gateway and agent state store
#[derive(Parser, Debug)]
#[command(name = "cade-server", version, about)]
struct ServerArgs {
    /// Port to listen on (overrides CADE_SERVER_PORT env var, default 8284)
    #[arg(long = "port", short = 'p', env = "CADE_SERVER_PORT", default_value_t = 8284)]
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

    // Inject the resolved port back into the environment so ServerConfig::from_env()
    // picks it up consistently (other code that reads CADE_SERVER_PORT also benefits).
    std::env::set_var("CADE_SERVER_PORT", args.port.to_string());

    let config = ServerConfig::from_env()?;

    tracing::info!(
        "CADE Server v{} | provider={} | db={}",
        env!("CARGO_PKG_VERSION"),
        config.llm_provider,
        config.db_path
    );

    let db = open_db(&config.db_path)?;

    // Build router from env vars first
    let mut router_inner = LlmRouter::build(&config);

    // Hot-load any providers persisted in the DB (DB overrides env vars)
    let db_providers = sqlite::list_providers(&db).unwrap_or_default();
    for row in &db_providers {
        if !row.enabled { continue; }
        if let Some(p) = LlmRouter::provider_from_row(row, &config) {
            // Store the API key so list_dynamic_models() can fetch live model lists.
            let key = row.api_key.clone().unwrap_or_default();
            router_inner.add_provider_with_key(row.name.clone(), p, key);
            tracing::info!("Loaded provider from DB: {} ({})", row.name, row.kind);
        }
    }

    tracing::info!("Active providers: {}", router_inner.provider_names().join(", "));

    let llm_router = Arc::new(RwLock::new(router_inner));
    // llm field: thin Arc pointing to the router itself (router implements LlmProvider)
    let llm: Arc<dyn cade::server::llm::LlmProvider> = {
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
    };

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
            tracing::info_span!("http", method = %req.method(), uri = %req.uri())
        })
        .on_failure(
            tower_http::trace::DefaultOnFailure::new().level(tracing::Level::ERROR)
        );

    let app = router(state)
        .layer(
            // H-03: Restrict CORS to localhost origins only (not permissive/open)
            CorsLayer::new()
                .allow_origin([
                    "http://localhost".parse::<HeaderValue>().unwrap(),
                    format!("http://localhost:{}", config.addr.port())
                        .parse::<HeaderValue>().unwrap(),
                    "http://127.0.0.1".parse::<HeaderValue>().unwrap(),
                    format!("http://127.0.0.1:{}", config.addr.port())
                        .parse::<HeaderValue>().unwrap(),
                ])
                .allow_methods([Method::GET, Method::POST, Method::PUT,
                                Method::PATCH, Method::DELETE, Method::OPTIONS])
                .allow_headers(tower_http::cors::Any),
        )
        .layer(trace_layer);

    tracing::info!("Listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── RouterAdapter: thin wrapper so Arc<RwLock<LlmRouter>> implements LlmProvider ──
//
// IMPORTANT: the lock is held ONLY for the brief resolve_provider() call.
// It is dropped BEFORE any HTTP calls to Anthropic / OpenAI / Gemini.
//
// Holding the lock across async HTTP calls (the old pattern) caused
// Tokio's write-preferring RwLock to starve subsequent readers (e.g.
// validate_model in PATCH /agents/:id) whenever GET /v1/models queued
// a hot_sync write — blocking /model switches mid-stream.

struct RouterAdapter(Arc<RwLock<LlmRouter>>);

#[async_trait::async_trait]
impl cade::server::llm::LlmProvider for RouterAdapter {
    async fn complete(
        &self,
        req: &CompletionRequest,
    ) -> anyhow::Result<cade::server::llm::CompletionResponse> {
        // Acquire lock just long enough to clone the provider Arc.
        let (provider, bare_model) = {
            let router = self.0.read().await;
            router.resolve_provider(&req.model)?
        }; // ← lock released here, BEFORE the HTTP call
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.complete(&routed).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> anyhow::Result<std::pin::Pin<Box<dyn tokio_stream::Stream<
        Item = anyhow::Result<cade::server::llm::StreamChunk>
    > + Send>>> {
        // Same pattern: lock only for routing, drop before streaming HTTP call.
        let (provider, bare_model) = {
            let router = self.0.read().await;
            router.resolve_provider(&req.model)?
        }; // ← lock released here, BEFORE the streaming HTTP call
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.stream(&routed).await
    }
}

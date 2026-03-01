use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use axum::http::Request;

use cade::server::{
    api::router,
    config::ServerConfig,
    llm::LlmRouter,
    state::AppState,
    storage::{open as open_db, sqlite},
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let _ = dotenvy::dotenv();
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
            router_inner.add_provider(row.name.clone(), p);
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
    };

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
            tracing::info_span!("http", method = %req.method(), uri = %req.uri())
        })
        .on_failure(
            tower_http::trace::DefaultOnFailure::new().level(tracing::Level::ERROR)
        );

    let app = router(state)
        .layer(CorsLayer::permissive())
        .layer(trace_layer);

    tracing::info!("Listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── RouterAdapter: thin wrapper so Arc<RwLock<LlmRouter>> implements LlmProvider ──

struct RouterAdapter(Arc<RwLock<LlmRouter>>);

#[async_trait::async_trait]
impl cade::server::llm::LlmProvider for RouterAdapter {
    async fn complete(
        &self,
        req: &cade::server::llm::CompletionRequest,
    ) -> anyhow::Result<cade::server::llm::CompletionResponse> {
        self.0.read().await.complete(req).await
    }

    async fn stream(
        &self,
        req: &cade::server::llm::CompletionRequest,
    ) -> anyhow::Result<std::pin::Pin<Box<dyn tokio_stream::Stream<
        Item = anyhow::Result<cade::server::llm::StreamChunk>
    > + Send>>> {
        self.0.read().await.stream(req).await
    }
}

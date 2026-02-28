use anyhow::Result;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

// Bring in the server module tree from the main crate
use cade::server::{
    api::router,
    config::ServerConfig,
    llm::make_provider,
    state::AppState,
    storage::open as open_db,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Load .env if present
    let _ = dotenvy::dotenv();

    // Config from env
    let config = ServerConfig::from_env()?;

    tracing::info!(
        "CADE Server v{} | provider={} | db={}",
        env!("CARGO_PKG_VERSION"),
        config.llm_provider,
        config.db_path
    );

    // Storage
    let db = open_db(&config.db_path)?;

    // LLM provider
    let llm = make_provider(&config).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n\nHint: set ANTHROPIC_API_KEY / OPENAI_API_KEY / GOOGLE_API_KEY / OLLAMA_BASE_URL\n\
             and CADE_LLM_PROVIDER=anthropic|openai|gemini|ollama"
        )
    })?;

    let state = AppState {
        db,
        llm,
        config: Arc::new(config.clone()),
    };

    // Build axum app
    let app = router(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    tracing::info!("Listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

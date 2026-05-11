// region:    --- Modules

mod error;
pub mod fetch;
pub mod search;
pub mod tools;

pub use error::{Error, Result};
pub use fetch::{FetchedDoc, fetch_doc};
pub use search::{SearchResult, web_search};
pub use tools::{BrowserScreenshotTool, FetchDocTool, WebSearchTool};

// endregion: --- Modules

pub static SHARED_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("CADE/0.2 (+https://github.com/EzekTec-Inc/CADE)")
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("Failed to build HTTP client")
});

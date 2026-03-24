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

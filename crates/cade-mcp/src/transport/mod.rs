//! Transport adapters for MCP servers (Stdio, Streamable HTTP, SSE).

pub mod http;
pub mod stdio;

pub use http::HttpTransportAdapter;
pub use stdio::{SingletonProcessGuard, StdioTransportAdapter};

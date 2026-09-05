//! Streamable HTTP & SSE Transport Adapter.
//!
//! Connects to remote MCP servers over HTTP/HTTPS with auto-negotiation
//! of Streamable HTTP vs SSE, bearer auth tokens, and custom header interpolation.

// region:    --- Imports

use http::{HeaderName, HeaderValue};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt, service::RunningService};
use std::collections::HashMap;
use tracing::info;

use crate::{Error, Result};
use cade_core::settings::McpServerConfig;

// endregion: --- Imports

// region:    --- HTTP Transport Adapter

pub struct HttpTransportAdapter;

impl HttpTransportAdapter {
    /// Connect to a remote MCP server via HTTP/HTTPS.
    pub async fn connect(
        key: &str,
        config: &McpServerConfig,
        url: &str,
    ) -> Result<(RunningService<RoleClient, ()>, rmcp::Peer<RoleClient>)> {
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);

        // 1. Inject Bearer token
        if let Some(token) = &config.auth_token {
            transport_config = transport_config.auth_header(format!("Bearer {token}"));
        }

        // 2. Inject custom headers with environment variable interpolation
        if let Some(custom_headers) = &config.headers {
            let mut headers = HashMap::new();
            for (k, v) in custom_headers {
                let header_name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                    Error::custom(format!("invalid header name '{k}' for '{key}': {e}"))
                })?;

                // Lightweight interpolation for `${VAR}` style
                let mut interpolated = v.to_string();
                while let Some(start) = interpolated.find("${") {
                    if let Some(end) = interpolated[start..].find('}') {
                        let end_idx = start + end;
                        let var_name = &interpolated[start + 2..end_idx];
                        let var_value = std::env::var(var_name).unwrap_or_default();
                        interpolated.replace_range(start..=end_idx, &var_value);
                    } else {
                        break;
                    }
                }

                let value = HeaderValue::from_str(&interpolated).map_err(|e| {
                    Error::custom(format!("invalid header value for '{k}' in '{key}': {e}"))
                })?;
                headers.insert(header_name, value);
            }
            transport_config = transport_config.custom_headers(headers);
        }

        info!("MCP server '{key}': connecting via HTTP → {url}");
        let transport = StreamableHttpClientTransport::from_config(transport_config);
        let service: RunningService<RoleClient, ()> = ()
            .serve(transport)
            .await
            .map_err(|e| Error::custom(format!("HTTP handshake with '{key}' ({url}): {e}")))?;
        let peer = service.peer().clone();

        Ok((service, peer))
    }
}

// endregion: --- HTTP Transport Adapter

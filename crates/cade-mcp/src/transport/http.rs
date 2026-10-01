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
            // rmcp applies the Bearer scheme when building the HTTP request.
            transport_config = transport_config.auth_header(token.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn authentication_sends_one_bearer_prefix() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let capture = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 1024];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8(bytes).unwrap();
            let auth = request
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("authorization")
                        .then(|| value.trim().to_owned())
                })
                .unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            auth
        });
        let config = McpServerConfig {
            url: Some(url.clone()),
            auth_token: Some("synthetic-test-token".into()),
            ..Default::default()
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            HttpTransportAdapter::connect("auth-test", &config, &url),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert_eq!(capture.await.unwrap(), "Bearer synthetic-test-token");
    }
}

//! Vsock Wire Protocol Types & Length-Prefixed Duplex Framing Engine (Issue #96 / PRD #95).

use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Structured request sent from host to guest daemon over vsock.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuestRequest {
    /// Execute a shell command inside the guest VM.
    Exec {
        command: String,
        cwd: String,
        timeout_secs: u64,
    },
    /// Read file content from guest filesystem.
    ReadFile { path: String },
    /// Write file content to guest filesystem.
    WriteFile { path: String, content: String },
    /// Check if path exists in guest filesystem.
    PathExists { path: String },
    /// List directory contents in guest filesystem.
    ListDir { path: String },
    /// Import in-memory tarball into `/workspace`.
    ImportWorkspaceTarball { tarball_bytes: Vec<u8> },
    /// Export `/workspace` as in-memory tarball.
    ExportWorkspaceTarball,
    /// Ping daemon for liveness check.
    Ping,
}

/// Structured response returned by guest daemon to host over vsock.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuestResponse {
    /// Result of an executed shell command.
    ExecResult {
        stdout: String,
        stderr: String,
        exit_code: i32,
        timed_out: bool,
    },
    /// File content read from guest.
    FileContent { content: String },
    /// Confirmation of file write.
    WriteOk,
    /// Path existence check outcome.
    PathExistsResult { exists: bool },
    /// Directory listing entries (path, is_dir, size).
    ListDirResult { entries: Vec<GuestDirEntry> },
    /// Exported in-memory tarball data.
    WorkspaceTarball { tarball_bytes: Vec<u8> },
    /// Confirmation of imported workspace.
    ImportWorkspaceOk,
    /// Pong liveness reply.
    Pong,
    /// Error message.
    Error { message: String },
}

/// Single directory entry inside the guest VM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuestDirEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

/// Send a length-prefixed request over an async writer.
pub async fn send_request<W: AsyncWrite + Unpin>(
    writer: &mut W,
    req: &GuestRequest,
) -> io::Result<()> {
    let payload = serde_json::to_vec(req)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Receive a length-prefixed request over an async reader.
pub async fn recv_request<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<GuestRequest> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Send a length-prefixed response over an async writer.
pub async fn send_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    resp: &GuestResponse,
) -> io::Result<()> {
    let payload = serde_json::to_vec(resp)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Receive a length-prefixed response over an async reader.
pub async fn recv_response<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<GuestResponse> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_duplex_request_response_roundtrip() -> io::Result<()> {
        let (mut client_stream, mut server_stream) = tokio::io::duplex(1024);

        let req = GuestRequest::Exec {
            command: "echo 'hello vsock'".to_string(),
            cwd: "/workspace".to_string(),
            timeout_secs: 10,
        };

        // Client sends request
        send_request(&mut client_stream, &req).await?;

        // Server receives request
        let server_received = recv_request(&mut server_stream).await?;
        assert_eq!(server_received, req);

        // Server replies with response
        let resp = GuestResponse::ExecResult {
            stdout: "hello vsock\n".to_string(),
            stderr: String::new(),
            exit_code: 0,
            timed_out: false,
        };
        send_response(&mut server_stream, &resp).await?;

        // Client receives response
        let client_received = recv_response(&mut client_stream).await?;
        assert_eq!(client_received, resp);
        Ok(())
    }

    #[tokio::test]
    async fn test_tarball_exchange_roundtrip() -> io::Result<()> {
        let (mut client_stream, mut server_stream) = tokio::io::duplex(2048);

        let tarball_bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let req = GuestRequest::ImportWorkspaceTarball {
            tarball_bytes: tarball_bytes.clone(),
        };

        send_request(&mut client_stream, &req).await?;
        let recvd = recv_request(&mut server_stream).await?;
        assert_eq!(recvd, req);

        let resp = GuestResponse::ImportWorkspaceOk;
        send_response(&mut server_stream, &resp).await?;
        let recvd_resp = recv_response(&mut client_stream).await?;
        assert_eq!(recvd_resp, resp);
        Ok(())
    }
}

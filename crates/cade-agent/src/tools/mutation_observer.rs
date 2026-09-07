//! File Mutation Observer Seam for Live Tracking.
//!
//! Emits structured file mutation events from tool executions to UI listeners.

// region:    --- Imports

use std::path::PathBuf;

// endregion: --- Imports

// region:    --- Types

/// Event emitted when a file has been modified or created by a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutationEvent {
    pub path: PathBuf,
    pub pre_content: String,
    pub post_content: String,
    pub tool_name: String,
}

/// Thread-safe observer trait for receiving file mutations.
#[async_trait::async_trait]
pub trait FileMutationObserver: Send + Sync {
    async fn on_file_mutated(&self, event: FileMutationEvent);
}

/// Channel-based mutation sender.
pub type MutationSender = tokio::sync::mpsc::UnboundedSender<FileMutationEvent>;
pub type MutationReceiver = tokio::sync::mpsc::UnboundedReceiver<FileMutationEvent>;

pub struct ChannelMutationObserver {
    tx: MutationSender,
}

impl ChannelMutationObserver {
    pub fn new(tx: MutationSender) -> Self {
        Self { tx }
    }
}

#[async_trait::async_trait]
impl FileMutationObserver for ChannelMutationObserver {
    async fn on_file_mutated(&self, event: FileMutationEvent) {
        let _ = self.tx.send(event);
    }
}

// endregion: --- Types

//! Advisory evaluation adapters for tool pre-screening.

pub mod jev;

pub use jev::{JevAdvisor, JevAdvisorConfig};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use async_trait::async_trait;
use cade_core::permissions::{AdvisoryReport, AdvisoryRequest, ToolAdvisor};

/// A mock advisor for unit and integration testing.
#[derive(Debug, Default, Clone)]
pub struct MockAdvisor {
    report_to_return: Option<AdvisoryReport>,
    call_count: Arc<AtomicUsize>,
}

impl MockAdvisor {
    pub fn new(report: Option<AdvisoryReport>) -> Self {
        Self {
            report_to_return: report,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ToolAdvisor for MockAdvisor {
    async fn advise(&self, _request: &AdvisoryRequest<'_>) -> Option<AdvisoryReport> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.report_to_return.clone()
    }
}

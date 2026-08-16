use std::sync::Arc;

use url::Url;

use super::protocol_support::ScriptExecutionReport;
use crate::renderer::{RendererPageState, RendererPerformanceMetricSnapshot};

pub(super) struct PageStateCache {
    state: Arc<RendererPageState>,
}

impl PageStateCache {
    pub(super) fn new(state: Arc<RendererPageState>) -> Self {
        Self { state }
    }

    pub(super) fn replace(&mut self, state: Arc<RendererPageState>) {
        self.state = state;
    }

    pub(super) fn state(&self) -> &RendererPageState {
        self.state.as_ref()
    }

    pub(super) fn requested_url(&self) -> &Url {
        &self.state().requested_url
    }

    pub(super) fn final_url(&self) -> &Url {
        self.state().final_url()
    }

    pub(super) fn document_title(&self) -> &str {
        self.state().document_title()
    }

    pub(super) fn status(&self) -> u16 {
        self.state().status
    }

    pub(super) fn navigation_initiator_url(&self) -> Option<&Url> {
        self.state().navigation_initiator_url.as_ref()
    }

    pub(super) fn navigation_redirected(&self) -> bool {
        self.state().navigation_redirected
    }

    pub(super) fn navigation_redirect_count(&self) -> usize {
        self.state().navigation_redirect_count
    }

    pub(super) fn headers(&self) -> &[(String, String)] {
        &self.state().headers
    }

    pub(super) fn script_execution(&self) -> &ScriptExecutionReport {
        &self.state().script_execution
    }

    pub(super) fn lifecycle_errors(&self) -> &[String] {
        self.state().script_execution.lifecycle_errors()
    }

    pub(super) fn performance_metric_snapshot(&self) -> &RendererPerformanceMetricSnapshot {
        &self.state().performance_metric_snapshot
    }
}

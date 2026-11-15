use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererChildFrameNavigationOutcome {
    Committed,
    Aborted,
    Download,
    Failed(String),
}

/// The result of one admitted child navigation, independent of later navigations.
/// Only Native admission, commit and cancellation paths settle this observation.
#[derive(Clone, Debug)]
pub struct RendererChildFrameNavigation {
    frame_id: String,
    loader_id: String,
    outcome: Arc<OnceLock<RendererChildFrameNavigationOutcome>>,
}

impl RendererChildFrameNavigation {
    pub(crate) fn new(frame_id: String, loader_id: String) -> Self {
        Self {
            frame_id,
            loader_id,
            outcome: Arc::new(OnceLock::new()),
        }
    }

    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub fn outcome(&self) -> Option<RendererChildFrameNavigationOutcome> {
        self.outcome.get().cloned()
    }

    pub(crate) fn commit(&self) {
        self.finish(RendererChildFrameNavigationOutcome::Committed);
    }

    pub(crate) fn abort(&self) {
        self.finish(RendererChildFrameNavigationOutcome::Aborted);
    }

    pub(crate) fn finish(&self, outcome: RendererChildFrameNavigationOutcome) {
        // A later replacement must not change a navigation that already committed.
        let _ = self.outcome.set(outcome);
    }
}

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererChildFrameNavigationOutcome {
    Committed,
    Aborted,
}

/// The result of one admitted child navigation, independent of later navigations.
/// Only Native admission, commit and cancellation paths settle this observation.
#[derive(Clone, Debug)]
pub struct RendererChildFrameNavigation {
    frame_id: String,
    loader_id: String,
    outcome: Arc<AtomicU8>,
}

impl RendererChildFrameNavigation {
    pub(crate) fn new(frame_id: String, loader_id: String) -> Self {
        Self {
            frame_id,
            loader_id,
            outcome: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub fn outcome(&self) -> Option<RendererChildFrameNavigationOutcome> {
        match self.outcome.load(Ordering::Acquire) {
            1 => Some(RendererChildFrameNavigationOutcome::Committed),
            2 => Some(RendererChildFrameNavigationOutcome::Aborted),
            _ => None,
        }
    }

    pub(crate) fn commit(&self) {
        self.settle(1);
    }

    pub(crate) fn abort(&self) {
        self.settle(2);
    }

    fn settle(&self, outcome: u8) {
        // A later replacement must not change a navigation that already committed.
        let _ = self
            .outcome
            .compare_exchange(0, outcome, Ordering::AcqRel, Ordering::Acquire);
    }
}

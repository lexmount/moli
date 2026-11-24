use crate::native_bridge::WindowDocumentTaskTarget;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Command validation shared by repeats; no input semantics live here.
#[derive(Clone, Debug, Default)]
pub struct RendererSyntheticWheelGesture {
    anchor_validated: Arc<AtomicBool>,
    document: Arc<Mutex<Option<WindowDocumentTaskTarget>>>,
}

impl RendererSyntheticWheelGesture {
    pub fn next_sequence(&self) -> Self {
        self.clone()
    }

    pub(crate) fn validate_anchor(&self, in_bounds: bool) -> bool {
        if in_bounds {
            self.anchor_validated.store(true, Ordering::Relaxed);
            true
        } else {
            self.anchor_validated.load(Ordering::Relaxed)
        }
    }

    pub(crate) fn accepts_document(&self, document: WindowDocumentTaskTarget) -> bool {
        let mut owner = self.document.lock();
        *owner.get_or_insert(document) == document
    }
}

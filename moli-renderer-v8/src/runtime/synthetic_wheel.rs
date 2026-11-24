use crate::native_bridge::WindowDocumentTaskTarget;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Bounds validation spans the command. A round binds its own Document on dispatch.
#[derive(Clone, Debug, Default)]
pub struct RendererSyntheticWheelGesture {
    anchor_validated: Arc<AtomicBool>,
    document: Arc<Mutex<Option<WindowDocumentTaskTarget>>>,
}

impl RendererSyntheticWheelGesture {
    pub fn next_sequence(&self) -> Self {
        Self {
            anchor_validated: Arc::clone(&self.anchor_validated),
            document: Arc::default(),
        }
    }

    pub(crate) fn validate_anchor(&self, in_bounds: bool) -> bool {
        if in_bounds {
            self.anchor_validated.store(true, Ordering::Relaxed);
            true
        } else {
            self.anchor_validated.load(Ordering::Relaxed)
        }
    }

    pub(crate) fn accepts_document(
        &self,
        document: WindowDocumentTaskTarget,
        bind_if_unset: bool,
    ) -> bool {
        let mut owner = self.document.lock();
        if bind_if_unset {
            owner.get_or_insert(document);
        }
        *owner == Some(document)
    }
}

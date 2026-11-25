use crate::native_bridge::WindowDocumentTaskTarget;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Bounds validation spans the command. A round binds its own Document on dispatch.
#[derive(Clone, Debug, Default)]
pub struct RendererSyntheticWheelGesture {
    anchor_validated: Arc<AtomicBool>,
    round: Arc<SyntheticWheelRound>,
}

#[derive(Debug)]
struct SyntheticWheelRound {
    id: u64,
    document: Mutex<Option<WindowDocumentTaskTarget>>,
}

impl Default for SyntheticWheelRound {
    fn default() -> Self {
        static NEXT_ROUND: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT_ROUND.fetch_add(1, Ordering::Relaxed),
            document: Mutex::default(),
        }
    }
}

impl RendererSyntheticWheelGesture {
    pub fn next_sequence(&self) -> Self {
        Self {
            anchor_validated: Arc::clone(&self.anchor_validated),
            round: Arc::default(),
        }
    }

    pub(crate) fn round_id(&self) -> u64 {
        self.round.id
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
        let mut owner = self.round.document.lock();
        if bind_if_unset {
            owner.get_or_insert(document);
        }
        *owner == Some(document)
    }
}

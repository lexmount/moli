use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use parking_lot::Mutex;

use crate::{
    document_runtime::DomHandle,
    native_bridge::{WindowDocumentTaskTarget, element::WheelScrollTarget},
};

/// One mouse-wheel sequence, shared only with its in-flight renderer commands.
/// A new value begins a sequence; dropping it ends the sequence, including when
/// the protocol workflow is canceled. No renderer-global target survives it.
#[derive(Clone, Debug, Default)]
pub struct RendererSyntheticWheelGesture {
    pub(crate) targets: Arc<Mutex<SyntheticWheelTargets>>,
    anchor_validated: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SyntheticWheelTargets {
    pub(crate) document: Option<WindowDocumentTaskTarget>,
    pub(crate) wheel: Option<(DomHandle, WindowDocumentTaskTarget)>,
    pub(crate) ancestors: Vec<DomHandle>,
    pub(crate) scroll_target: Option<WheelScrollTarget>,
}

impl RendererSyntheticWheelGesture {
    /// Start a repeat with fresh targets, retaining the command's initial anchor
    /// validation so a later viewport resize only skips out-of-bounds updates.
    pub fn next_sequence(&self) -> Self {
        Self {
            targets: Arc::default(),
            anchor_validated: Arc::clone(&self.anchor_validated),
        }
    }

    /// Re-latch after a direction change while retaining the gesture's Document.
    pub fn restart_phase(&self) -> Self {
        let restarted = self.next_sequence();
        restarted.targets.lock().document = self.targets.lock().document;
        restarted
    }

    pub(crate) fn validate_anchor(&self, in_bounds: bool) -> bool {
        if in_bounds {
            self.anchor_validated.store(true, Ordering::Relaxed);
            true
        } else {
            self.anchor_validated.load(Ordering::Relaxed)
        }
    }
}

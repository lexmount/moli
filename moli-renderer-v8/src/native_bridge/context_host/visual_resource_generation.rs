#[cfg(feature = "molio")]
use parking_lot::RwLock;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[cfg(feature = "molio")]
use std::sync::{OnceLock, Weak};
#[cfg(feature = "molio")]
use tokio::sync::Notify;

/// Shared generation for immutable resources sampled by one paint pass.
///
/// HTML images, CSS images, and canvases all contribute pixels to the same
/// visual output. Keeping one cloneable atomic lets asynchronous decoders and
/// owner-thread mutations publish through the same screencast race fence.
#[derive(Clone, Default)]
pub(crate) struct VisualResourceGeneration(Arc<VisualResourceState>);

#[derive(Default)]
struct VisualResourceState {
    generation: AtomicU64,
    #[cfg(feature = "molio")]
    viewport_wake: OnceLock<RwLock<Weak<Notify>>>,
}

impl VisualResourceGeneration {
    #[cfg(any(test, feature = "devtools"))]
    pub(crate) fn current(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }

    pub(crate) fn bump(&self) {
        self.0.generation.fetch_add(1, Ordering::AcqRel);
        #[cfg(feature = "molio")]
        if let Some(wake) = self
            .0
            .viewport_wake
            .get()
            .and_then(|wake| wake.read().upgrade())
        {
            wake.notify_one();
        }
    }

    #[cfg(feature = "molio")]
    pub(crate) fn set_viewport_wake(&self, wake: &Arc<Notify>) {
        *self.0.viewport_wake.get_or_init(RwLock::default).write() = Arc::downgrade(wake);
    }
}

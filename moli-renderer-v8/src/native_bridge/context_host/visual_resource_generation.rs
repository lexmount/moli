use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Shared generation for immutable resources sampled by one paint pass.
///
/// HTML images, CSS images, and canvases all contribute pixels to the same
/// visual output. Keeping one cloneable atomic lets asynchronous decoders and
/// owner-thread mutations publish through the same screencast race fence.
#[derive(Clone, Default)]
pub(crate) struct VisualResourceGeneration(Arc<AtomicU64>);

impl VisualResourceGeneration {
    pub(crate) fn current(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn bump(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

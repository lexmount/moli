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
    #[cfg(feature = "devtools")]
    retained_paint_generation: AtomicU64,
    #[cfg(all(feature = "molio", target_os = "macos"))]
    composited_video_generation: AtomicU64,
    #[cfg(feature = "molio")]
    viewport_wake: OnceLock<RwLock<Weak<Notify>>>,
}

impl VisualResourceGeneration {
    #[cfg(any(test, feature = "devtools"))]
    pub(crate) fn current(&self) -> u64 {
        let current = self.0.generation.load(Ordering::Acquire);
        #[cfg(all(feature = "molio", target_os = "macos"))]
        let current =
            current.wrapping_add(self.0.composited_video_generation.load(Ordering::Acquire));
        current
    }

    pub(crate) fn bump(&self) {
        #[cfg(feature = "devtools")]
        self.0
            .retained_paint_generation
            .fetch_add(1, Ordering::AcqRel);
        self.bump_visual_output();
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn retained_paint_generation(&self) -> u64 {
        self.0.retained_paint_generation.load(Ordering::Acquire)
    }

    /// CSS images are bound afresh when painting a retained screen.
    pub(crate) fn bump_css_image(&self) {
        self.bump_visual_output();
    }

    fn bump_visual_output(&self) {
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

    #[cfg(all(feature = "media-ext", feature = "molio", target_os = "macos"))]
    pub(super) fn bump_composited_video(&self) {
        self.0
            .retained_paint_generation
            .fetch_add(1, Ordering::AcqRel);
        self.0
            .composited_video_generation
            .fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(all(feature = "molio", target_os = "macos"))]
    pub(super) fn viewport_current(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }

    #[cfg(feature = "molio")]
    pub(crate) fn set_viewport_wake(&self, wake: &Arc<Notify>) {
        *self.0.viewport_wake.get_or_init(RwLock::default).write() = Arc::downgrade(wake);
    }
}

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

#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct VisualResourceRevisions {
    pub(super) other_pixels: u64,
    pub(super) css_images: u64,
}

#[cfg(any(test, feature = "devtools"))]
impl VisualResourceRevisions {
    pub(super) fn generation(self) -> u64 {
        self.other_pixels.saturating_add(self.css_images)
    }
}

#[derive(Default)]
struct VisualResourceState {
    other_pixels: AtomicU64,
    css_images: AtomicU64,
    #[cfg(feature = "molio")]
    viewport_wake: OnceLock<RwLock<Weak<Notify>>>,
}

impl VisualResourceGeneration {
    #[cfg(any(test, feature = "devtools"))]
    pub(crate) fn current(&self) -> u64 {
        self.revisions().generation()
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn revisions(&self) -> VisualResourceRevisions {
        VisualResourceRevisions {
            other_pixels: self.0.other_pixels.load(Ordering::Acquire),
            css_images: self.0.css_images.load(Ordering::Acquire),
        }
    }

    pub(crate) fn bump(&self) {
        self.publish(&self.0.other_pixels);
    }

    /// The CSS image store supplies background and mask pixels to paint.
    pub(super) fn bump_css_images(&self) {
        self.publish(&self.0.css_images);
    }

    fn publish(&self, revision: &AtomicU64) {
        revision.fetch_add(1, Ordering::AcqRel);
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

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Identity and lifetime of a script-created top-level browsing context.
/// Both its opener's reference and its own Page retain this handle.
#[derive(Clone, Debug)]
pub struct RendererAuxiliaryWindow(Arc<AuxiliaryWindowState>);

#[derive(Debug)]
struct AuxiliaryWindowState {
    id: u64,
    closed: AtomicBool,
    navigation_cancellation_generation: AtomicU64,
    is_popup: AtomicBool,
    frame_sandbox: crate::document_runtime::DocumentSandboxPolicy,
}

impl RendererAuxiliaryWindow {
    pub(super) fn new(
        id: u64,
        frame_sandbox: crate::document_runtime::DocumentSandboxPolicy,
    ) -> Self {
        Self(Arc::new(AuxiliaryWindowState {
            id,
            closed: AtomicBool::new(false),
            navigation_cancellation_generation: AtomicU64::new(0),
            is_popup: AtomicBool::new(false),
            frame_sandbox,
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    fn advance_navigation_cancellation_generation(&self) -> u64 {
        self.0
            .navigation_cancellation_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            })
            .expect("auxiliary navigation generation exhausted")
            + 1
    }

    pub(crate) fn pending_navigation_generation(&self) -> u64 {
        self.0
            .navigation_cancellation_generation
            .load(Ordering::Acquire)
    }

    pub(crate) fn cancel_pending_navigation(&self) {
        self.advance_navigation_cancellation_generation();
    }

    /// A renderer can stop this request before the browser has
    /// adopted its initial Page. Preserve that decision across projection.
    pub fn accepts_pending_navigation(&self, generation: u64) -> bool {
        self.0
            .navigation_cancellation_generation
            .load(Ordering::Acquire)
            == generation
            && !self.is_closed()
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
    }

    /// Popup UI state belongs to the browsing context, including after its
    /// initial Document is transferred from the opener to an independent Page.
    pub(crate) fn is_popup(&self) -> bool {
        self.0.is_popup.load(Ordering::Acquire)
    }

    pub(crate) fn set_is_popup(&self, is_popup: bool) {
        self.0.is_popup.store(is_popup, Ordering::Release);
    }

    pub(crate) fn frame_sandbox(&self) -> crate::document_runtime::DocumentSandboxPolicy {
        self.0.frame_sandbox
    }

    /// Returns true only for the first accepted close request.
    pub fn close(&self) -> bool {
        !self.0.closed.swap(true, Ordering::AcqRel)
    }
}

impl PartialEq for RendererAuxiliaryWindow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererAuxiliaryWindow {}

/// The Location operation accepted for an existing auxiliary context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererAuxiliaryNavigationKind {
    Assign,
    Replace,
    Reload,
}

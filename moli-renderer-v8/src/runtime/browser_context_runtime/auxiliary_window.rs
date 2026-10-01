use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Identity and lifetime of a script-created top-level browsing context.
/// Both its opener's reference and its own Page retain this handle.
#[derive(Clone, Debug)]
pub struct RendererAuxiliaryWindow(Arc<AuxiliaryWindowState>);

#[derive(Debug)]
struct AuxiliaryWindowState {
    id: u64,
    closed: AtomicBool,
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
            frame_sandbox,
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
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

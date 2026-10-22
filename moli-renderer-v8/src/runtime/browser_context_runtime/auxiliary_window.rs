use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Identity and lifetime of a script-created top-level browsing context.
/// Both its opener's reference and its own Page retain this handle.
#[derive(Clone, Debug)]
pub struct RendererAuxiliaryWindow(Arc<AuxiliaryWindowState>);

#[derive(Debug)]
struct AuxiliaryWindowState {
    id: u64,
    close_state: AtomicU8,
    is_popup: AtomicBool,
    frame_sandbox: crate::document_runtime::DocumentSandboxPolicy,
}

#[repr(u8)]
enum AuxiliaryWindowCloseState {
    Open,
    CheckingBeforeUnload,
    Requested,
    Closed,
}

/// Claims the synchronous beforeunload check without retiring its Document.
/// Dropping an unfinished check releases only this claim, never a forced close.
#[must_use]
pub(crate) struct RendererAuxiliaryWindowCloseGuard {
    window: RendererAuxiliaryWindow,
}

impl RendererAuxiliaryWindowCloseGuard {
    pub(crate) fn finish(self) -> bool {
        self.window
            .0
            .close_state
            .compare_exchange(
                AuxiliaryWindowCloseState::CheckingBeforeUnload as u8,
                AuxiliaryWindowCloseState::Requested as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

impl Drop for RendererAuxiliaryWindowCloseGuard {
    fn drop(&mut self) {
        let _ = self.window.0.close_state.compare_exchange(
            AuxiliaryWindowCloseState::CheckingBeforeUnload as u8,
            AuxiliaryWindowCloseState::Open as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl RendererAuxiliaryWindow {
    pub(super) fn new(
        id: u64,
        frame_sandbox: crate::document_runtime::DocumentSandboxPolicy,
    ) -> Self {
        Self(Arc::new(AuxiliaryWindowState {
            id,
            close_state: AtomicU8::new(AuxiliaryWindowCloseState::Open as u8),
            is_popup: AtomicBool::new(false),
            frame_sandbox,
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn is_closed(&self) -> bool {
        matches!(
            self.0.close_state.load(Ordering::Acquire),
            state if state == AuxiliaryWindowCloseState::Requested as u8
                || state == AuxiliaryWindowCloseState::Closed as u8
        )
    }

    pub(crate) fn is_open(&self) -> bool {
        self.0.close_state.load(Ordering::Acquire) == AuxiliaryWindowCloseState::Open as u8
    }

    pub(crate) fn is_closing(&self) -> bool {
        matches!(
            self.0.close_state.load(Ordering::Acquire),
            state if state == AuxiliaryWindowCloseState::CheckingBeforeUnload as u8
                || state == AuxiliaryWindowCloseState::Requested as u8
        )
    }

    pub(crate) fn begin_close(&self) -> Option<RendererAuxiliaryWindowCloseGuard> {
        self.0
            .close_state
            .compare_exchange(
                AuxiliaryWindowCloseState::Open as u8,
                AuxiliaryWindowCloseState::CheckingBeforeUnload as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| RendererAuxiliaryWindowCloseGuard {
                window: self.clone(),
            })
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

    /// Claims final retirement, including forced teardown without beforeunload.
    /// Returns true once even if a script already requested a queued close.
    pub fn close(&self) -> bool {
        self.0
            .close_state
            .swap(AuxiliaryWindowCloseState::Closed as u8, Ordering::AcqRel)
            != AuxiliaryWindowCloseState::Closed as u8
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

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> RendererAuxiliaryWindow {
        RendererAuxiliaryWindow::new(1, Default::default())
    }

    #[test]
    fn close_check_rejects_reentry_without_closing_the_window() {
        let window = window();
        let retained = window.clone();
        let check = window.begin_close().unwrap();
        assert!(!retained.is_open());
        assert!(retained.is_closing());
        assert!(!retained.is_closed());
        assert!(retained.begin_close().is_none());

        assert!(check.finish());
        assert!(retained.is_closed());
        assert!(retained.is_closing());
        assert!(retained.begin_close().is_none());
        assert!(window.close());
        assert!(!retained.is_closing());
        assert!(!retained.close());
    }

    #[test]
    fn abandoned_close_check_releases_only_its_claim() {
        let window = window();
        drop(window.begin_close().unwrap());
        assert!(window.is_open());
        let check = window.begin_close().unwrap();
        assert!(window.close());
        drop(check);
        assert!(window.is_closed());
        assert!(window.begin_close().is_none());
    }

    #[test]
    fn forced_close_during_check_cannot_be_reopened_or_requeued() {
        let window = window();
        let check = window.begin_close().unwrap();
        let retained = window.clone();
        assert!(std::thread::spawn(move || retained.close()).join().unwrap());
        assert!(!check.finish());
        assert!(window.is_closed());
        assert!(window.begin_close().is_none());
        assert!(!window.close());
    }
}

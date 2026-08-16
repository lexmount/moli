use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Thread-safe script-execution policy shared by a stable Page residence and
/// whichever Document runtime is currently installed in that Page.
///
/// DevTools may update this setting from its IO agent while the renderer owner
/// is executing JavaScript. Publishing the policy must therefore neither
/// borrow the active `PageVm` nor wait for the owner lane.
#[derive(Clone, Debug)]
pub(crate) struct RendererScriptExecutionControl {
    disabled: Arc<AtomicBool>,
}

impl RendererScriptExecutionControl {
    pub(crate) fn new(disabled: bool) -> Self {
        Self {
            disabled: Arc::new(AtomicBool::new(disabled)),
        }
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Acquire)
    }

    pub(crate) fn set_disabled(&self, disabled: bool) {
        self.disabled.store(disabled, Ordering::Release);
    }
}

impl Default for RendererScriptExecutionControl {
    fn default() -> Self {
        Self::new(false)
    }
}

//! The old document's part of preparing a replacement document.
//!
//! Capturing a replacement is inert. The renderer preparation boundary starts
//! the scope before queuing owner work, since the owner may be inside V8's pause
//! loop. Prepared-document ownership keeps it alive until commit or cancellation.

use std::sync::Arc;

use crate::devtools::pause::RendererInspectorPauseBridge;

#[derive(Clone, Debug)]
pub struct RendererDocumentReplacement {
    pause: RendererInspectorPauseBridge,
    cancellation: moli_fetch::FetchCancelHandle,
}

impl RendererDocumentReplacement {
    pub(super) fn new(
        pause: RendererInspectorPauseBridge,
        cancellation: moli_fetch::FetchCancelHandle,
    ) -> Self {
        Self {
            pause,
            cancellation,
        }
    }

    pub(super) fn begin(self) -> anyhow::Result<Arc<RendererDocumentReplacementScope>> {
        anyhow::ensure!(
            !self.cancellation.is_cancelled() && self.pause.begin_document_replacement(),
            moli_fetch::NET_ERR_ABORTED_ERROR_TEXT,
        );
        Ok(Arc::new(RendererDocumentReplacementScope {
            pause: self.pause,
        }))
    }
}

/// Owned by the prepare command and its returned handle, not by protocol or by
/// clones of the inert replacement input. Overlapping prepares for one old
/// document remain independent; completing either one must not release the other.
pub(crate) struct RendererDocumentReplacementScope {
    pause: RendererInspectorPauseBridge,
}

impl Drop for RendererDocumentReplacementScope {
    fn drop(&mut self) {
        self.pause.finish_document_replacement();
    }
}

#[cfg(test)]
mod tests;

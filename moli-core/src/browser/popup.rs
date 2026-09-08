use std::sync::{Arc, Weak};

use super::{DocumentHandle, RendererPageResidenceIdentity, WebContentsHandle};
use crate::page::{RendererPopupOpening, RendererPopupOpeningId, RendererWindowDocumentSource};

/// Immutable creation provenance. Window/opener identity is native; DevTools
/// chooses its own Target and frame projection when the renderer FIFO arrives.
#[derive(Clone, Debug)]
pub struct BrowserPopupCreation {
    pub request: RendererPopupOpeningId,
    pub source_document: DocumentHandle,
    pub source_renderer: RendererPageResidenceIdentity,
    pub source_window: Option<RendererWindowDocumentSource>,
    pub opener: Option<WebContentsHandle>,
    pub requested_url: String,
    pub(crate) renderer_opening: Weak<RendererPopupOpening>,
}

impl BrowserPopupCreation {
    /// The live renderer observation owns this reference, not Browser history.
    /// Its notifications retain their original FIFO position until consumed.
    pub fn pending_renderer_opening(&self) -> Option<Arc<RendererPopupOpening>> {
        self.renderer_opening.upgrade()
    }
}

impl PartialEq for BrowserPopupCreation {
    fn eq(&self, other: &Self) -> bool {
        self.request == other.request
    }
}

impl Eq for BrowserPopupCreation {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserPopupAdmission {
    pub source_document: DocumentHandle,
    pub web_contents: WebContentsHandle,
    pub created: bool,
}

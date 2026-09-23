use crate::document_runtime::DomHandle;
use crate::frame_owner_model::DocumentId;
use crate::runtime::RendererDocumentLifecycleIdentity;

/// A preparation is bound to live renderer identities, never to a protocol
/// frame id or a JavaScript status string.
#[derive(Debug, Clone, PartialEq)]
pub enum RendererElementClickTarget {
    DomActivation,
    Option,
    FileInput,
    Pointer(RendererPreparedPointerClick),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererPreparedPointerClick {
    pub root_x: f64,
    pub root_y: f64,
    pub(crate) target: DomHandle,
    pub(crate) root_document: Option<RendererDocumentLifecycleIdentity>,
    pub(crate) documents: Vec<(DomHandle, DocumentId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererElementClickError {
    StaleNode,
    MissingBrowsingContext,
    NoClickableRect,
    Obscured,
    LayoutUnavailable(String),
}

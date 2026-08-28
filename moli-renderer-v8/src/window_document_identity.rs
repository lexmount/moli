use crate::frame_owner_model::FrameDocumentTaskOwner;

/// Exact identity of one live Window `Document`, independent of its dispatch
/// address and of any numeric projection used by a transport.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowDocumentOwner {
    Frame(FrameDocumentTaskOwner),
}

impl WindowDocumentOwner {
    pub(crate) fn frame_document_owner(self) -> Option<FrameDocumentTaskOwner> {
        match self {
            Self::Frame(owner) => Some(owner),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(identity: u64) -> Self {
        Self::Frame(FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(identity),
            crate::frame_owner_model::LocalWindowId(identity),
            crate::frame_owner_model::DocumentId(identity),
        ))
    }
}

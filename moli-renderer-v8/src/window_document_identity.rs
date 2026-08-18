use crate::frame_owner_model::FrameDocumentTaskOwner;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupDocumentId(u64);

impl LightweightPopupDocumentId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupLocalWindowId(u64);

impl LightweightPopupLocalWindowId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupDocumentOwner {
    popup_id: u64,
    document_id: LightweightPopupDocumentId,
}

impl LightweightPopupDocumentOwner {
    pub(crate) fn new(popup_id: u64, document_id: LightweightPopupDocumentId) -> Self {
        Self {
            popup_id,
            document_id,
        }
    }

    pub(crate) fn popup_id(self) -> u64 {
        self.popup_id
    }

    pub(crate) fn document_id(self) -> LightweightPopupDocumentId {
        self.document_id
    }
}

/// Exact identity of one live Window `Document`, independent of its dispatch
/// address and of any numeric projection used by a transport.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowDocumentOwner {
    Frame(FrameDocumentTaskOwner),
    LightweightPopup(LightweightPopupDocumentOwner),
}

impl WindowDocumentOwner {
    pub(crate) fn frame_document_owner(self) -> Option<FrameDocumentTaskOwner> {
        match self {
            Self::Frame(owner) => Some(owner),
            Self::LightweightPopup(_) => None,
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

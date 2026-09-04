use crate::{PageId, RendererOutputResidenceIdentity, RendererOwnerLocalHostId, page::Page};

/// Exact renderer Page backing a browser Document.
///
/// The owner and Page ids reject work from a replaced Page. Renderer document
/// epochs remain separate: `document.open()` restarts the lifecycle inside the
/// same physical Page rather than replacing this residence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererPageResidenceIdentity {
    owner_local_host_id: RendererOwnerLocalHostId,
    page_id: PageId,
}

impl RendererPageResidenceIdentity {
    pub const fn from_parts(
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
    ) -> Self {
        Self {
            owner_local_host_id,
            page_id,
        }
    }

    pub fn from_page(page: &Page) -> Self {
        Self::from_parts(page.renderer_owner_local_host_id(), page.renderer_page_id())
    }

    pub const fn owner_local_host_id(self) -> RendererOwnerLocalHostId {
        self.owner_local_host_id
    }

    pub const fn page_id(self) -> PageId {
        self.page_id
    }

    pub const fn from_residence(residence: RendererOutputResidenceIdentity) -> Option<Self> {
        match residence {
            RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            } => Some(Self::from_parts(owner_local_host_id, page_id)),
            RendererOutputResidenceIdentity::SharedWorker { .. }
            | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
        }
    }

    pub fn matches_residence(self, residence: RendererOutputResidenceIdentity) -> bool {
        Self::from_residence(residence) == Some(self)
    }
}

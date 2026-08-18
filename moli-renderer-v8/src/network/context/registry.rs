use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::native_bridge::WindowDocumentOwner;

use super::DocumentResourceLoader;

/// Exact owner-to-authority index for all Documents hosted by one Page VM.
///
/// It is intentionally separate from the shared browser resource runtime: a
/// frame handle may be reused after navigation, while the full
/// `FrameDocumentTaskOwner` includes the non-reused Document identity.
#[derive(Clone, Debug, Default)]
pub(crate) struct DocumentResourceLoaderRegistry {
    loaders: Rc<RefCell<HashMap<WindowDocumentOwner, DocumentResourceLoader>>>,
}

impl DocumentResourceLoaderRegistry {
    pub(crate) fn register(&self, owner: WindowDocumentOwner, loader: DocumentResourceLoader) {
        assert!(
            self.loaders.borrow_mut().insert(owner, loader).is_none(),
            "Document resource loader owner registered twice: {owner:?}"
        );
    }

    /// Replaces only the request-client view of an already registered
    /// authority.
    ///
    /// Browser backend adoption can rebuild the transport used by a live
    /// Document, but it must not manufacture a second lifecycle authority for
    /// the same owner. Updating the indexed wrapper keeps future child
    /// inheritance and request snapshots on the replacement backend.
    pub(crate) fn replace_transport_view(
        &self,
        owner: WindowDocumentOwner,
        loader: DocumentResourceLoader,
    ) {
        let mut loaders = self.loaders.borrow_mut();
        let registered = loaders
            .get_mut(&owner)
            .unwrap_or_else(|| panic!("cannot replace transport for unknown Document: {owner:?}"));
        assert!(
            registered.shares_authority_with(&loader),
            "Document owner cannot be rebound to a second resource authority: {owner:?}"
        );
        *registered = loader;
    }

    pub(crate) fn get(&self, owner: WindowDocumentOwner) -> Option<DocumentResourceLoader> {
        self.loaders
            .borrow()
            .get(&owner)
            .filter(|loader| loader.accepts_ordinary_loads())
            .cloned()
    }

    pub(crate) fn retire(&self, owner: WindowDocumentOwner) -> Option<DocumentResourceLoader> {
        let loader = self.loaders.borrow_mut().remove(&owner)?;
        loader.begin_detach();
        loader.finish_detach();
        Some(loader)
    }

    pub(crate) fn retire_all(&self) -> Vec<DocumentResourceLoader> {
        self.loaders
            .borrow_mut()
            .drain()
            .map(|(_, loader)| {
                loader.begin_detach();
                loader.finish_detach();
                loader
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.loaders.borrow().len()
    }
}

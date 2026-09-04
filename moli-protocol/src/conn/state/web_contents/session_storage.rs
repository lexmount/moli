use std::sync::OnceLock;

use moli_core::network::{
    SharedWebStorageStore, deep_clone_shared_web_storage_store, new_shared_web_storage_store,
};

#[derive(Default)]
pub(crate) struct SessionStorageNamespace {
    store: OnceLock<SharedWebStorageStore>,
}

impl std::fmt::Debug for SessionStorageNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStorageNamespace")
            .field("initialized", &self.store.get().is_some())
            .finish()
    }
}

impl SessionStorageNamespace {
    pub(crate) fn from_store(store: SharedWebStorageStore) -> Self {
        Self {
            store: OnceLock::from(store),
        }
    }

    pub(crate) fn store(&self) -> &SharedWebStorageStore {
        self.store.get_or_init(new_shared_web_storage_store)
    }

    pub(crate) fn deep_clone(&self) -> Self {
        Self {
            store: OnceLock::from(deep_clone_shared_web_storage_store(self.store())),
        }
    }
}

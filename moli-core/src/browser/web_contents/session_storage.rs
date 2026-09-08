use std::sync::OnceLock;

use crate::network::{
    SharedWebStorageStore, deep_clone_shared_web_storage_store, new_shared_web_storage_store,
};

#[derive(Default)]
pub struct SessionStorageNamespace {
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
    pub fn from_store(store: SharedWebStorageStore) -> Self {
        Self {
            store: OnceLock::from(store),
        }
    }

    pub fn store(&self) -> &SharedWebStorageStore {
        self.store.get_or_init(new_shared_web_storage_store)
    }

    pub fn deep_clone(&self) -> Self {
        Self {
            store: OnceLock::from(deep_clone_shared_web_storage_store(self.store())),
        }
    }
}

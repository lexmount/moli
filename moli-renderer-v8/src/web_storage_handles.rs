use std::{fmt, sync::Arc};

use crate::{SharedWebStorageStore, new_shared_web_storage_store};
use parking_lot::Mutex;

/// State owned by one top-level browsing context and shared by each Document
/// committed into it.
#[derive(Clone, Default)]
pub struct RendererTopLevelWindowName {
    value: Arc<Mutex<String>>,
}

impl RendererTopLevelWindowName {
    pub(crate) fn get(&self) -> String {
        self.value.lock().clone()
    }

    pub(crate) fn set(&self, value: String) {
        *self.value.lock() = value;
    }
}

impl fmt::Debug for RendererTopLevelWindowName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererTopLevelWindowName")
            .field("strong_count", &Arc::strong_count(&self.value))
            .finish_non_exhaustive()
    }
}

/// Web Storage state installed into one renderer page environment.
///
/// This type is intentionally independent from the network `ResourceRequestClient`.
/// `localStorage` belongs to the storage partition and `sessionStorage`
/// belongs to the browsing context; rebuilding the network backend must not
/// replace either store.
#[derive(Clone)]
pub struct RendererWebStorageHandles {
    local_storage: SharedWebStorageStore,
    session_storage: SharedWebStorageStore,
    top_level_window_name: RendererTopLevelWindowName,
}

impl RendererWebStorageHandles {
    pub fn new(
        local_storage: SharedWebStorageStore,
        session_storage: SharedWebStorageStore,
    ) -> Self {
        Self {
            local_storage,
            session_storage,
            top_level_window_name: RendererTopLevelWindowName::default(),
        }
    }

    pub fn with_top_level_window_name(mut self, value: RendererTopLevelWindowName) -> Self {
        self.top_level_window_name = value;
        self
    }

    pub fn ephemeral() -> Self {
        Self::new(
            new_shared_web_storage_store(),
            new_shared_web_storage_store(),
        )
    }

    pub fn local_storage(&self) -> SharedWebStorageStore {
        self.local_storage.clone()
    }

    pub fn session_storage(&self) -> SharedWebStorageStore {
        self.session_storage.clone()
    }

    pub(crate) fn top_level_window_name(&self) -> RendererTopLevelWindowName {
        self.top_level_window_name.clone()
    }

    pub fn shares_local_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.local_storage, &other.local_storage)
    }

    pub fn shares_session_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.session_storage, &other.session_storage)
    }
}

impl Default for RendererWebStorageHandles {
    fn default() -> Self {
        Self::ephemeral()
    }
}

impl fmt::Debug for RendererWebStorageHandles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererWebStorageHandles")
            .field(
                "local_storage_strong_count",
                &Arc::strong_count(&self.local_storage),
            )
            .field(
                "session_storage_strong_count",
                &Arc::strong_count(&self.session_storage),
            )
            .finish()
    }
}

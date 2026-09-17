use super::IndexedDbManager;
pub(crate) use moli_indexeddb::{
    ConnectionRequestHandle, ConnectionRequestLease, ConnectionRequestWake,
};
use parking_lot::Mutex;
use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, Weak},
};

pub type SharedIndexedDbManager = Arc<Mutex<IndexedDbManager>>;

#[derive(Clone)]
pub struct WeakIndexedDbManager {
    inner: Weak<Mutex<IndexedDbManager>>,
}

impl WeakIndexedDbManager {
    pub(crate) fn upgrade(&self) -> Option<SharedIndexedDbManager> {
        self.inner.upgrade()
    }

    pub(crate) fn close_database_handles(
        &self,
        handles: impl IntoIterator<Item = moli_indexeddb::DatabaseHandle>,
    ) -> usize {
        let Some(manager) = self.upgrade() else {
            return 0;
        };
        let mut manager = manager.lock();
        handles
            .into_iter()
            .filter(|handle| manager.force_close_database(*handle).is_ok())
            .count()
    }
}

impl fmt::Debug for WeakIndexedDbManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakIndexedDbManager")
            .field("is_alive", &self.inner.strong_count().gt(&0))
            .finish()
    }
}

pub fn new_indexed_db_manager(
    root: Option<PathBuf>,
) -> std::result::Result<SharedIndexedDbManager, String> {
    let manager = match root {
        Some(path) => IndexedDbManager::new(path).map_err(|error| error.to_string())?,
        None => IndexedDbManager::new_in_memory(),
    };
    Ok(Arc::new(Mutex::new(manager)))
}

pub fn downgrade_indexed_db_manager(manager: &SharedIndexedDbManager) -> WeakIndexedDbManager {
    WeakIndexedDbManager {
        inner: Arc::downgrade(manager),
    }
}

pub fn clear_indexed_db_origin(
    manager: &SharedIndexedDbManager,
    origin: &str,
) -> std::result::Result<(), String> {
    manager
        .lock()
        .clear_origin(origin)
        .map_err(|error| error.to_string())
}

pub fn clear_indexed_db_origins_with_prefix(
    manager: &SharedIndexedDbManager,
    origin_prefix: &str,
) -> std::result::Result<(), String> {
    manager
        .lock()
        .clear_origins_with_prefix(origin_prefix)
        .map_err(|error| error.to_string())
}

pub fn indexed_db_origin_usage_bytes(
    manager: &SharedIndexedDbManager,
    origin: &str,
) -> std::result::Result<u64, String> {
    manager
        .lock()
        .origin_usage_bytes(origin)
        .map_err(|error| error.to_string())
}

pub fn indexed_db_origins_with_prefix_usage_bytes(
    manager: &SharedIndexedDbManager,
    origin_prefix: &str,
) -> std::result::Result<u64, String> {
    manager
        .lock()
        .origins_with_prefix_usage_bytes(origin_prefix)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bootstrap::indexed_db::{
        Key, ObjectStoreOptions, OpenOptions, TransactionMode,
    };

    #[test]
    fn weak_indexed_db_manager_expires_with_last_strong_owner() {
        let weak = {
            let manager = new_indexed_db_manager(None)
                .expect("in-memory indexedDB manager should initialize");
            let weak = downgrade_indexed_db_manager(&manager);
            assert!(weak.upgrade().is_some());
            weak
        };

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn indexed_db_origin_usage_reads_shared_manager_usage() {
        let manager =
            new_indexed_db_manager(None).expect("in-memory IndexedDB manager should initialize");
        let origin = "https://usage-helper.example";
        {
            let mut manager = manager.lock();
            let opened = manager
                .open(OpenOptions {
                    origin: origin.to_owned(),
                    name: "app".to_owned(),
                    version: None,
                })
                .expect("open should succeed");
            let upgrade = opened
                .upgrade_transaction
                .expect("upgrade transaction should exist");
            manager
                .create_object_store(upgrade, "items", ObjectStoreOptions::default())
                .expect("store should be created");
            manager
                .commit_transaction(upgrade)
                .expect("upgrade commit should succeed");
            let tx = manager
                .begin_transaction(
                    opened.database,
                    &["items".into()],
                    TransactionMode::ReadWrite,
                )
                .expect("readwrite transaction should start");
            manager
                .put(tx, "items", Some(Key::from("alpha")), b"first".to_vec())
                .expect("put should succeed");
            manager
                .commit_transaction(tx)
                .expect("record transaction should commit");
        }

        assert!(
            indexed_db_origin_usage_bytes(&manager, origin).expect("usage should be readable") > 0
        );
        assert_eq!(
            indexed_db_origin_usage_bytes(&manager, "https://other.example")
                .expect("other origin usage should be readable"),
            0
        );
    }
}

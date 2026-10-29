use super::IndexedDbManager;
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
    new_sqlite_indexed_db_manager(root, Default::default())
}

/// Initialize and recover storage before starting its Page/Worker event loops.
pub fn new_sqlite_indexed_db_manager(
    root: Option<PathBuf>,
    config: moli_indexeddb::sqlite::service::ServiceConfig,
) -> std::result::Result<SharedIndexedDbManager, String> {
    use moli_indexeddb::sqlite::service::Service;
    let service = match root {
        Some(root) => Service::on_disk(root, config).map_err(|error| error.to_string())?,
        None => Service::new(config).map_err(|error| error.to_string())?,
    };
    Ok(Arc::new(Mutex::new(IndexedDbManager::with_sqlite_service(
        service,
    ))))
}

pub fn downgrade_indexed_db_manager(manager: &SharedIndexedDbManager) -> WeakIndexedDbManager {
    WeakIndexedDbManager {
        inner: Arc::downgrade(manager),
    }
}

/// Host-side cleanup, called outside V8. Native SQL work runs on storage workers.
pub fn clear_indexed_db_origin(
    manager: &SharedIndexedDbManager,
    origin: &str,
) -> std::result::Result<(), String> {
    let sql = manager.lock().sqlite_service();
    sql.prepare_clear_owner(origin.as_bytes().to_vec())
        .and_then(|clear| clear.execute())
        .map_err(|error| error.to_string())
}

/// Host-side cleanup; the scope fence also excludes newly created owners.
pub fn clear_indexed_db_origins_with_prefix(
    manager: &SharedIndexedDbManager,
    origin_prefix: &str,
) -> std::result::Result<(), String> {
    let sql = manager.lock().sqlite_service();
    sql.prepare_clear_owners_with_prefix(origin_prefix.as_bytes().to_vec())
        .and_then(|clear| clear.execute())
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
    use crate::context_bootstrap::indexed_db::{Key, ObjectStoreOptions, TransactionMode};

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
            use moli_indexeddb::sqlite::service::Command;
            let service = manager.lock().sqlite_service();
            let (sender, receiver) = std::sync::mpsc::channel();
            let client = service
                .resolve(origin.as_bytes().to_vec(), "app".into(), move |reply| {
                    sender.send(reply).unwrap();
                })
                .unwrap();
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .result
                .unwrap();
            let execute = |command| {
                let (sender, receiver) = std::sync::mpsc::channel();
                client
                    .submit(command, 0, move |reply| {
                        sender.send(reply).unwrap();
                    })
                    .unwrap();
                receiver
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .result
                    .unwrap()
            };
            let transaction = client.allocate_transaction_handle().unwrap();
            execute(Command::Begin {
                transaction,
                mode: TransactionMode::VersionChange,
                stores: Default::default(),
                version: Some(1),
                admission: None,
            });
            execute(Command::CreateStore {
                transaction,
                name: "items".into(),
                options: ObjectStoreOptions::default(),
            });
            execute(Command::Write {
                transaction,
                store: "items".into(),
                key: Some(Key::from("alpha")),
                value: b"first".to_vec().into(),
                indexes: vec![],
                add_only: false,
                quota: None,
            });
            assert_eq!(indexed_db_origin_usage_bytes(&manager, origin).unwrap(), 0);
            execute(Command::Commit {
                transaction,
                quota: None,
            });
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

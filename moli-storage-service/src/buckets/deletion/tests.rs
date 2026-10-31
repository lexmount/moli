use super::*;
use moli_indexeddb::{
    ObjectStoreOptions, TransactionHandle, TransactionMode,
    sqlite::service::{
        Command, CommandResult, DatabaseClient, Service, ServiceClient, ServiceConfig,
    },
};
use std::{collections::BTreeSet, sync::mpsc, time::Duration};

const STORAGE_KEY: &str = "bucket-delete-test";
const TX: TransactionHandle = TransactionHandle::from_raw(1);

struct Fixture {
    store: SharedStorageBucketStore,
    identity: StorageBucketIdentity,
    sql: ServiceClient,
    _manager: SharedStorageBucketIndexedDbManager,
}

impl Fixture {
    fn new() -> Self {
        let config = ServiceConfig::default();

        let manager = Arc::new(Mutex::new(IndexedDbManager::with_sqlite_service(
            Service::new(config).unwrap(),
        )));
        let sql = manager.lock().sqlite_service();
        let store = new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
        let identity = store.lock().open_bucket(STORAGE_KEY, "remove").unwrap();
        Self {
            store,
            identity,
            sql,
            _manager: manager,
        }
    }

    fn database(&self, identity: &StorageBucketIdentity) -> DatabaseClient {
        let (sender, receiver) = mpsc::channel();
        let client = self
            .sql
            .resolve(
                identity.indexed_db_storage_key().into_bytes(),
                "records".into(),
                move |reply| {
                    let _ = sender.send(reply);
                },
            )
            .unwrap();
        receive(&receiver).result.unwrap();
        execute(
            &client,
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
                durability: moli_indexeddb::TransactionDurability::Strict,
                stores: BTreeSet::new(),
                version: Some(1),
                admission: None,
            },
        );
        execute(
            &client,
            Command::CreateStore {
                transaction: TX,
                name: "records".into(),
                options: ObjectStoreOptions::default(),
            },
        );
        execute(
            &client,
            Command::Write {
                transaction: TX,
                store: "records".into(),
                key: Some(1.into()),
                value: b"kept".to_vec().into(),
                indexes: vec![],
                add_only: false,
                quota: None,
            },
        );
        execute(
            &client,
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        );
        client
    }

    fn revoke(&self) {
        assert_eq!(
            self.store
                .lock()
                .delete_bucket(STORAGE_KEY, "remove")
                .unwrap(),
            Some(self.identity.clone())
        );
    }

    fn prepare(&self) -> SqlBucketDeletion {
        prepare_sql_bucket_deletion(&self.store, &self.identity)
            .unwrap()
            .unwrap()
    }

    fn dispatch(&self) -> mpsc::Receiver<SqlBucketDeletionResult> {
        let (sender, receiver) = mpsc::channel();
        self.prepare()
            .dispatch(move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receiver
    }

    fn block_worker(&self) -> Gate {
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        self.store
            .lock()
            .storage_service()
            .maintenance_queue
            .dispatch(
                move || {
                    entered.send(()).unwrap();
                    receive(&resume);
                },
                |result| result.unwrap(),
            )
            .unwrap();
        receive(&waiting);
        Gate(Some(release))
    }
}

struct Gate(Option<mpsc::Sender<()>>);
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("bucket deletion must progress after its explicit gate opens")
}

fn execute(client: &DatabaseClient, command: Command) -> CommandResult {
    let (sender, receiver) = mpsc::channel();
    client
        .submit(command, move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver).result.unwrap()
}

#[test]
fn deletion_waits_for_quota_without_blocking_sql_and_releases_results_after_delivery() {
    let fixture = Fixture::new();
    let client = fixture.database(&fixture.identity);
    let storage = fixture.store.lock().storage_service();
    let locator = fixture.identity.locator();
    let key = StorageService::opfs_bucket_key(&locator).unwrap();
    let root = storage.ensure_opfs_root(&locator).unwrap();
    let file = storage
        .with_opfs(|opfs| opfs.get_file(&key, &root, "remove.txt", true))
        .unwrap();
    storage
        .with_opfs(|opfs| opfs.write_file(&key, &file, b"remove", None))
        .unwrap();
    let quota = storage.reserve_quota_commit(&locator);
    fixture.revoke();
    let reply = fixture.dispatch();
    // The maintenance worker can wait for quota while the SQL lane still runs.
    let admission = client
        .queue_transaction_start(
            moli_indexeddb::DatabaseHandle::from_raw(7),
            &["records".into()],
            TransactionMode::ReadOnly,
            Arc::new(|| {}),
        )
        .unwrap();
    execute(
        &client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::ReadOnly,
            durability: moli_indexeddb::TransactionDurability::Relaxed,
            stores: BTreeSet::from(["records".into()]),
            version: None,
            admission: Some(admission.handle().clone()),
        },
    );
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: TX,
                store: "records".into(),
                source: Default::default(),
                range: Default::default(),
            }
        ),
        CommandResult::Count(1)
    ));
    assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(!client.is_closed());
    drop(quota);
    let reply = receive(&reply);
    assert!(reply.result.is_ok(), "{:?}", reply.result);
    assert!(client.is_retired());
    assert!(fixture.store.lock().pending_deletions().is_empty());
    assert_eq!(
        fixture
            .sql
            .owner_usage_bytes(fixture.identity.indexed_db_storage_key().as_bytes())
            .unwrap(),
        0
    );
    assert_eq!(storage.opfs_usage(&locator).unwrap(), 0);
    assert!(storage.quota_coordinator.try_reserve(&locator).is_some());
}

#[test]
fn queued_duplicate_deletion_cannot_erase_a_same_name_replacement() {
    let fixture = Fixture::new();
    let old = fixture.database(&fixture.identity);
    fixture.revoke();
    let gate = fixture.block_worker();
    let (first, first_reply) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let callback_gate = Gate(Some(release));
    fixture
        .prepare()
        .dispatch(move |reply| {
            let _ = first.send(reply);
            receive(&resume);
        })
        .unwrap();
    let duplicate = fixture.dispatch();

    assert!(!old.is_closed());
    drop(gate);
    assert!(receive(&first_reply).result.is_ok());
    let replacement = fixture
        .store
        .lock()
        .open_bucket(STORAGE_KEY, "remove")
        .unwrap();
    assert_ne!(replacement.bucket_id(), fixture.identity.bucket_id());
    let fresh = fixture.database(&replacement);
    drop(callback_gate);
    assert!(receive(&duplicate).result.is_ok());
    assert!(old.is_retired());
    assert!(!fresh.is_closed());
    assert_eq!(fresh.committed().unwrap().version, 1);
    assert!(
        fixture
            .sql
            .owner_usage_bytes(replacement.indexed_db_storage_key().as_bytes())
            .unwrap()
            > 0
    );
}

#[test]
fn dropping_unaccepted_deletion_leaves_a_retryable_tombstone() {
    let fixture = Fixture::new();
    let client = fixture.database(&fixture.identity);
    fixture.revoke();
    let work = fixture.prepare();

    drop(work);

    assert!(!client.is_closed());
    assert_eq!(
        fixture.store.lock().pending_deletions(),
        std::slice::from_ref(&fixture.identity)
    );
    assert!(receive(&fixture.dispatch()).result.is_ok());
    assert!(client.is_retired());
}

#[test]
fn deletion_retires_the_database_after_receiving_cleanup_acknowledgement() {
    let fixture = Fixture::new();
    let client = fixture.database(&fixture.identity);
    fixture.revoke();

    let work = prepare_sql_bucket_deletion(&fixture.store, &fixture.identity)
        .unwrap()
        .expect("SQL bucket deletion");
    assert!(!client.is_closed());
    assert_eq!(
        fixture.store.lock().pending_deletions(),
        std::slice::from_ref(&fixture.identity)
    );
    let (sender, receiver) = mpsc::channel();
    work.dispatch(move |reply| {
        let _ = sender.send(reply);
    })
    .unwrap();
    assert!(receive(&receiver).result.is_ok());
    assert!(client.is_retired());
}

#[test]
fn closed_service_deletion_keeps_retryable_tombstone() {
    let fixture = Fixture::new();
    let client = fixture.database(&fixture.identity);
    let other = fixture
        .store
        .lock()
        .open_bucket(STORAGE_KEY, "foreign")
        .unwrap();
    let foreign = fixture.database(&other);
    fixture.revoke();
    let gate = fixture.block_worker();
    let reply = fixture.dispatch();
    let stopped = std::mem::replace(
        &mut *fixture._manager.lock(),
        IndexedDbManager::with_sqlite_service(Service::new(Default::default()).unwrap()),
    );
    drop(stopped);
    drop(gate);
    assert!(receive(&reply).result.is_err());
    assert_eq!(
        fixture.store.lock().pending_deletions(),
        std::slice::from_ref(&fixture.identity)
    );
    assert!(
        fixture
            .store
            .lock()
            .open_bucket(STORAGE_KEY, "remove")
            .is_err()
    );
    // Shutdown retires both SQL clients, but never deletes the other identity.
    assert!(client.is_retired() && foreign.is_retired());
    assert_eq!(
        fixture.store.lock().bucket_identity(STORAGE_KEY, "foreign"),
        Some(other)
    );
}

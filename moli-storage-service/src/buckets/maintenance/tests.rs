use super::*;
use moli_indexeddb::{
    ObjectStoreOptions, TransactionHandle, TransactionMode,
    sqlite::service::{Command, DatabaseClient, Service, ServiceConfig},
};
use std::{
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const OWNER: &str = "maintenance-owner";
const TX: TransactionHandle = TransactionHandle::from_raw(1);

struct Fixture {
    root: PathBuf,
    manager: SharedStorageBucketIndexedDbManager,
    store: SharedStorageBucketStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "moli-bucket-maintenance-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let manager = Self::manager(&root);
        let store = Self::open_store(&root, &manager);
        Self {
            root,
            manager,
            store,
        }
    }

    fn manager(root: &Path) -> SharedStorageBucketIndexedDbManager {
        let mut config = ServiceConfig::default();
        config.executor.workers = 1;
        Arc::new(Mutex::new(IndexedDbManager::with_sqlite_service(
            Service::on_disk(root.join("idb-lite"), config).unwrap(),
        )))
    }

    fn open_store(
        root: &Path,
        manager: &SharedStorageBucketIndexedDbManager,
    ) -> SharedStorageBucketStore {
        new_shared_json_storage_bucket_store_with_storage_service(
            root.join("buckets.json"),
            root.join("cache"),
            manager,
            StorageService::on_disk(root.join("opfs")).unwrap(),
        )
        .unwrap()
    }

    fn sql(&self) -> ServiceClient {
        self.manager.lock().sqlite_service().unwrap()
    }

    fn dispatch(&self, operation: SqlBucketOperation) -> mpsc::Receiver<SqlBucketOperationResult> {
        let (sender, receiver) = mpsc::channel();
        prepare_sql_bucket_maintenance(&self.store, operation)
            .unwrap()
            .unwrap()
            .dispatch(move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receiver
    }

    fn run(&self, operation: SqlBucketOperation) -> SqlBucketOperationValue {
        receive(&self.dispatch(operation)).result.unwrap()
    }

    fn open(&self, name: &str) -> StorageBucketIdentity {
        let SqlBucketOperationValue::Opened(identity) = self.run(SqlBucketOperation::Open {
            storage_key: OWNER.into(),
            name: name.into(),
            expires: None,
            durability: None,
            quota: None,
            persisted: None,
        }) else {
            panic!("expected an opened bucket")
        };
        identity
    }

    fn seed(&self, owner: &[u8]) -> DatabaseClient {
        let (sender, receiver) = mpsc::channel();
        let client = self
            .sql()
            .resolve(owner.to_vec(), "records".into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receive(&receiver).result.unwrap();
        for command in [
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
                stores: BTreeSet::new(),
                version: Some(1),
                admission: None,
            },
            Command::CreateStore {
                transaction: TX,
                name: "kept".into(),
                options: ObjectStoreOptions::default(),
            },
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        ] {
            let (sender, receiver) = mpsc::channel();
            client
                .submit(command, 0, move |reply| {
                    let _ = sender.send(reply);
                })
                .unwrap();
            receive(&receiver).result.unwrap();
        }
        client
    }

    fn expire(&self, identity: &StorageBucketIdentity) {
        assert!(matches!(
            self.run(SqlBucketOperation::SetExpires {
                identity: identity.clone(),
                expires: Some(1.0)
            }),
            SqlBucketOperationValue::Complete
        ));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.store
            .lock()
            .storage_service()
            .drain_maintenance()
            .unwrap();
        self.manager.lock().shutdown_sqlite().unwrap();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("maintenance must finish after its gate opens")
}

struct Gate(Option<mpsc::Sender<()>>);
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[test]
fn bucket_creation_is_queued_before_identity_publication_and_leaves_sql_available() {
    let fixture = Fixture::new();
    let storage = fixture.store.lock().storage_service();
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let gate = Gate(Some(release));
    storage
        .quota_wait_queue
        .dispatch(
            move || {
                entered.send(()).unwrap();
                receive(&resume);
            },
            |reply| reply.unwrap(),
        )
        .unwrap();
    receive(&waiting);
    let reply = fixture.dispatch(SqlBucketOperation::Open {
        storage_key: OWNER.into(),
        name: "new".into(),
        expires: None,
        durability: None,
        quota: None,
        persisted: None,
    });
    assert!(!fixture.root.join("buckets.json").exists());
    assert!(fixture.store.lock().bucket_identity(OWNER, "new").is_none());
    assert!(matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let unrelated = fixture.seed(b"unrelated");
    assert_eq!(unrelated.committed().unwrap().version, 1);
    let reserved = fixture.sql().metadata_bytes();
    drop(gate);
    let reply = receive(&reply);
    let SqlBucketOperationValue::Opened(identity) = reply.result.as_ref().unwrap() else {
        panic!("expected opened bucket")
    };
    assert_eq!(
        fixture.store.lock().bucket_identity(OWNER, "new").as_ref(),
        Some(identity)
    );
    assert!(fixture.root.join("buckets.json").exists());
    assert!(fixture.sql().metadata_bytes() >= reserved);
    drop(reply);
    assert!(fixture.sql().metadata_bytes() < reserved);
}

#[test]
fn reopening_an_expired_bucket_clears_its_sql_generation_without_touching_other_owners() {
    let fixture = Fixture::new();
    let old = fixture.open("expired");
    let client = fixture.seed(old.indexed_db_storage_key().as_bytes());
    let other = fixture.seed(b"unrelated");
    fixture.expire(&old);
    assert!(!fixture.store.lock().bucket_locator_is_live(&old.locator()));
    let replacement = fixture.open("expired");
    assert_ne!(replacement.bucket_id(), old.bucket_id());
    assert!(client.is_retired());
    assert!(!other.is_retired());
    assert_eq!(other.committed().unwrap().version, 1);
    assert!(fixture.store.lock().pending_deletions().is_empty());
    assert_eq!(
        fixture
            .sql()
            .owner_usage_bytes(old.indexed_db_storage_key().as_bytes())
            .unwrap(),
        0
    );
}

#[test]
fn keys_finishes_expired_cleanup_and_an_old_expiry_job_cannot_delete_a_replacement() {
    let fixture = Fixture::new();
    let old = fixture.open("expired");
    let client = fixture.seed(old.indexed_db_storage_key().as_bytes());
    fixture.open("kept");
    fixture.expire(&old);
    let result = fixture.run(SqlBucketOperation::Keys {
        storage_key: OWNER.into(),
    });
    let SqlBucketOperationValue::Keys { names, .. } = result else {
        panic!("expected keys")
    };
    assert_eq!(names, ["kept"]);
    assert!(client.is_retired());
    let replacement = fixture.open("expired");
    let kept = fixture.seed(replacement.indexed_db_storage_key().as_bytes());
    fixture.expire(&replacement);
    fixture.run(SqlBucketOperation::Expire { identity: old });
    assert_eq!(
        fixture.store.lock().bucket_identity(OWNER, "expired"),
        Some(replacement)
    );
    assert!(!kept.is_retired());
}

#[test]
fn sql_bucket_startup_replays_durable_tombstones_before_reusing_a_name() {
    let mut fixture = Fixture::new();
    let old = fixture.open("deleted");
    let client = fixture.seed(old.indexed_db_storage_key().as_bytes());
    fixture.seed(b"unrelated");
    fixture
        .store
        .lock()
        .delete_bucket(OWNER, "deleted")
        .unwrap();
    fixture.manager.lock().shutdown_sqlite().unwrap();
    assert!(client.is_retired());
    fixture.manager = Fixture::manager(&fixture.root);
    fixture.store = Fixture::open_store(&fixture.root, &fixture.manager);
    assert!(fixture.store.lock().pending_deletions().is_empty());
    assert_eq!(
        fixture
            .sql()
            .owner_usage_bytes(old.indexed_db_storage_key().as_bytes())
            .unwrap(),
        0
    );
    assert!(fixture.sql().owner_usage_bytes(b"unrelated").unwrap() > 0);
    assert!(fixture.open("deleted").bucket_id() > old.bucket_id());
}

#[test]
fn old_bucket_metadata_can_open_sql_without_importing_old_indexeddb_json() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("indexeddb"),
        b"unreadable legacy IndexedDB format",
    )
    .unwrap();
    fs::write(
        fixture.root.join("buckets.json"),
        br#"{
        "version":4,"nextBucketId":18,"origins":{"maintenance-owner":{"old":{"bucketId":17}}}
    }"#,
    )
    .unwrap();
    let store = Fixture::open_store(&fixture.root, &fixture.manager);
    assert_eq!(store.lock().bucket_id(OWNER, "old").unwrap().get(), 17);
    assert_eq!(
        fs::read(fixture.root.join("indexeddb")).unwrap(),
        b"unreadable legacy IndexedDB format"
    );
    assert_eq!(
        fixture.sql().owners_with_prefix_usage_bytes(b"").unwrap(),
        0
    );
}

use super::*;
use moli_browser_profile::BrowserProfile;
use moli_indexeddb::{
    ObjectStoreOptions, TransactionHandle, TransactionMode,
    sqlite::service::{Command, CommandResult, DatabaseClient, ServiceClient},
};
use std::{
    collections::BTreeSet,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const TX: TransactionHandle = TransactionHandle::from_raw(7);

fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("partition shutdown must progress after its explicit gate opens")
}

fn partition(root: &std::path::Path) -> StoragePartitionState {
    StoragePartitionState::open(Some(root)).unwrap()
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

fn database(service: &ServiceClient, owner: &[u8]) -> DatabaseClient {
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve(owner.to_vec(), "records".into(), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    receive(&receiver).result.unwrap();
    client
}

fn upgrade(client: &DatabaseClient, version: u64, store: &str) {
    execute(
        client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            durability: moli_indexeddb::TransactionDurability::Strict,
            stores: BTreeSet::new(),
            version: Some(version),
            admission: None,
        },
    );
    execute(
        client,
        Command::CreateStore {
            transaction: TX,
            name: store.into(),
            options: ObjectStoreOptions::default(),
        },
    );
}

struct Gate(Option<mpsc::Sender<()>>);
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[test]
fn partition_shutdown_retires_retained_sql_handles_before_releasing_the_profile_lock() {
    let root = TempProfileDir::new("sqlite-shutdown");
    let partition = partition(&root.path);
    let manager = partition.indexed_db_manager.clone();
    let service = manager.lock().sqlite_service();
    let client = database(&service, b"shutdown-owner");
    upgrade(&client, 1, "kept");
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    );
    upgrade(&client, 2, "discarded");
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let gate = Gate(Some(release));
    client
        .submit(
            Command::Count {
                transaction: TX,
                store: "kept".into(),
                source: Default::default(),
                range: Default::default(),
            },
            move |_| {
                entered.send(()).unwrap();
                receive(&resume);
            },
        )
        .unwrap();
    receive(&waiting);
    let (done, finished) = mpsc::channel();
    let owner = thread::spawn(move || {
        drop(partition);
        let _ = done.send(());
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !service.is_closed() {
        assert!(
            matches!(finished.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "profile was released while its retained SQL owner remained open"
        );
        assert!(
            Instant::now() < deadline,
            "partition must request SQL shutdown"
        );
        thread::yield_now();
    }
    assert!(
        BrowserProfile::open(&root.path).is_err(),
        "profile lock must cover pending SQL finalizers"
    );
    assert!(matches!(
        finished.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(gate);
    receive(&finished);
    owner.join().unwrap();
    assert!(client.is_retired());
    assert!(
        manager.lock().sqlite_service().is_closed(),
        "closed SQL owners must not revert to the legacy backend"
    );
    assert!(
        client
            .submit(Command::Abort { transaction: TX }, |_| {})
            .is_err()
    );
    // Retaining the old manager/client no longer pins SQLite files or the lock.
    let _profile = BrowserProfile::open(&root.path).unwrap();
    let reopened_manager = moli_renderer_v8::new_sqlite_indexed_db_manager(
        Some(root.path.join("partitions/default/idb-lite")),
        Default::default(),
    )
    .unwrap();
    let reopened = reopened_manager.lock().sqlite_service();
    let kept = database(&reopened, b"shutdown-owner");
    let metadata = kept.committed().unwrap();
    assert_eq!(metadata.version, 1);
    assert!(
        metadata
            .stores
            .contains_key(&moli_indexeddb::IndexedDbName::from("kept"))
    );
    assert!(
        !metadata
            .stores
            .contains_key(&moli_indexeddb::IndexedDbName::from("discarded"))
    );
}

#[test]
fn partition_shutdown_finishes_accepted_bucket_cleanup_before_closing_sql() {
    let root = TempProfileDir::new("sqlite-cleanup-shutdown");
    let partition = partition(&root.path);
    let store = partition.storage_bucket_store.clone();
    let identity = store
        .lock()
        .open_bucket("shutdown-owner", "remove")
        .unwrap();
    let service = partition.indexed_db_manager.lock().sqlite_service();
    let client = database(&service, identity.indexed_db_storage_key().as_bytes());
    upgrade(&client, 1, "kept");
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    );
    let quota = partition
        .storage_service
        .reserve_quota_commit(&identity.locator());
    store
        .lock()
        .delete_bucket(identity.storage_key(), identity.name())
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    moli_storage_service::prepare_sql_bucket_deletion(&store, &identity)
        .unwrap()
        .unwrap()
        .dispatch(move |result| {
            let _ = sender.send(result);
        })
        .unwrap();
    let (done, finished) = mpsc::channel();
    let owner = thread::spawn(move || {
        drop(partition);
        let _ = done.send(());
    });
    assert!(BrowserProfile::open(&root.path).is_err());
    assert!(matches!(
        finished.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(quota);
    assert!(
        receive(&receiver).result.is_ok(),
        "accepted cleanup must run while SQL is still available"
    );
    receive(&finished);
    owner.join().unwrap();
    assert!(store.lock().pending_deletions().is_empty());
    assert!(client.is_retired() && service.is_closed());
    let _profile = BrowserProfile::open(&root.path).unwrap();
}

#[test]
fn temporary_partition_stores_sql_on_disk_and_retires_retained_clients_before_cleanup() {
    let partition = StoragePartitionState::open(None).unwrap();
    let root = partition
        .temporary_directory
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    let service = partition.indexed_db_manager.lock().sqlite_service();
    let client = database(&service, b"default-memory");
    upgrade(&client, 1, "kept");
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    );
    let mut directories = vec![root.join("idb-lite")];
    let mut databases = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "sqlite")
            {
                databases.push(path);
            }
        }
    }
    assert_eq!(databases.len(), 1);
    assert!(
        fs::read(&databases[0])
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    drop(partition);
    assert!(service.is_closed());
    assert!(client.is_retired());
    assert!(
        !root.exists(),
        "retained SQL clients must not pin temporary files"
    );
    let partition = StoragePartitionState::open(None).unwrap();
    let service = partition.indexed_db_manager.lock().sqlite_service();
    assert_eq!(
        database(&service, b"default-memory")
            .committed()
            .unwrap()
            .version,
        0
    );
}

#[test]
fn default_profile_rejects_corrupt_sql_without_reading_or_replacing_legacy_data() {
    let root = TempProfileDir::new("sql-corruption");
    let partition = StoragePartitionState::open(Some(&root.path)).unwrap();
    let paths = moli_browser_profile::BrowserProfilePaths::new(&root.path);
    let legacy = paths.partition_root.join("indexeddb");
    fs::write(&legacy, b"legacy IDB must remain untouched").unwrap();
    drop(partition);
    fs::write(
        paths.indexeddb_root.join("format.json"),
        b"corrupt SQL marker",
    )
    .unwrap();
    assert!(StoragePartitionState::open(Some(&root.path)).is_err());
    assert_eq!(
        fs::read(&legacy).unwrap(),
        b"legacy IDB must remain untouched"
    );
    assert_eq!(
        fs::read(paths.indexeddb_root.join("format.json")).unwrap(),
        b"corrupt SQL marker"
    );
    let _profile = BrowserProfile::open(&root.path).unwrap();
}

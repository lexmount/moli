use super::*;
use moli_indexeddb::sqlite::service::{CommandResult, Service, ServiceConfig};
use moli_indexeddb::{ObjectStoreOptions, TransactionHandle, TransactionMode};
use std::{collections::BTreeSet, sync::mpsc, time::Duration};

const TX: TransactionHandle = TransactionHandle::from_raw(2);

struct Fixture {
    owner: StorageBucketQuotaOwner,
    _manager: SharedStorageBucketIndexedDbManager,
}

impl Fixture {
    fn new() -> Self {
        let mut config = ServiceConfig::default();
        config.executor.workers = 1;
        let manager = Arc::new(Mutex::new(IndexedDbManager::with_sqlite_service(
            Service::new(config).unwrap(),
        )));
        let store = new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
        let identity = store
            .lock()
            .open_bucket("test-storage-key", "shared")
            .unwrap();
        let owner = storage_bucket_quota_owner(&store, &identity.locator()).unwrap();
        Self {
            owner,
            _manager: manager,
        }
    }

    fn database(&self, name: &str) -> DatabaseClient {
        let locator = &self.owner.locator;
        let key = storage_bucket_indexed_db_storage_key(
            locator.storage_key(),
            locator.bucket_id().unwrap(),
        );
        let service = self._manager.lock().sqlite_service();
        let (sender, receiver) = mpsc::channel();
        let client = service
            .resolve(key.into_bytes(), name.into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receive(&receiver).result.unwrap();
        execute(
            &client,
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
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
            Command::Commit {
                transaction: TX,
                quota: None,
            },
        );
        execute(
            &client,
            Command::Begin {
                transaction: TX,
                mode: TransactionMode::VersionChange,
                stores: BTreeSet::new(),
                version: Some(2),
                admission: None,
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
        client
    }

    fn commit(
        &self,
        client: &DatabaseClient,
    ) -> mpsc::Receiver<Result<Completion, IndexedDbError>> {
        let command = Command::Commit {
            transaction: TX,
            quota: None,
        };
        let (sender, receiver) = mpsc::channel();
        self.owner
            .dispatch_indexed_db(client.clone(), command, move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receiver
    }

    fn block_quota_worker(&self) -> QueueGate {
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        self.owner
            .storage_service
            .quota_queue
            .dispatch(
                move || {
                    entered.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(10)).unwrap();
                },
                |result| result.unwrap(),
            )
            .unwrap();
        receive(&waiting);
        QueueGate(Some(release))
    }
}

struct QueueGate(Option<mpsc::Sender<()>>);
impl Drop for QueueGate {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("storage operation must progress after its explicit gate opens")
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
fn later_quota_submission_cannot_hold_the_token_needed_by_an_earlier_queued_commit() {
    let fixture = Fixture::new();
    let first = fixture.database("first");
    let second = fixture.database("second");
    let gate = fixture.block_quota_worker();
    let token = fixture.owner.reserve_commit();
    let first_reply = fixture.commit(&first);
    drop(token);
    let second_reply = fixture.commit(&second);
    drop(gate);
    for receiver in [first_reply, second_reply] {
        let reply = receive(&receiver).unwrap();
        assert!(matches!(reply.result, Ok(CommandResult::Metadata(_))));
        assert_eq!(reply.active_transaction, None);
    }
    assert_eq!(first.committed().unwrap().version, 2);
    assert_eq!(second.committed().unwrap().version, 2);
    // The reservation is released before publishing either completion.
    assert!(
        fixture
            .owner
            .storage_service
            .quota_coordinator
            .try_reserve(&fixture.owner.locator)
            .is_some()
    );
}

#[test]
fn waiting_for_bucket_quota_leaves_the_database_worker_free() {
    let fixture = Fixture::new();
    let waiting = fixture.database("waiting");
    let other = fixture.database("unrelated");
    let token = fixture.owner.reserve_commit();
    let completion = fixture.commit(&waiting);
    assert!(matches!(
        execute(
            &other,
            Command::Count {
                transaction: TX,
                store: "records".into(),
                source: Default::default(),
                range: Default::default(),
            }
        ),
        CommandResult::Count(1)
    ));
    assert!(matches!(
        completion.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    execute(&other, Command::Abort { transaction: TX });
    drop(token);
    receive(&completion).unwrap().result.unwrap();
}

#[test]
fn closing_a_database_during_quota_wait_reports_dispatch_failure_without_a_sql_receipt() {
    let fixture = Fixture::new();
    let client = fixture.database("closing");
    let gate = fixture.block_quota_worker();
    let completion = fixture.commit(&client);
    execute(&client, Command::Close);
    drop(gate);
    assert!(matches!(
        receive(&completion),
        Err(IndexedDbError::Resource(_))
    ));
    assert!(
        fixture
            .owner
            .storage_service
            .quota_coordinator
            .try_reserve(&fixture.owner.locator)
            .is_some()
    );
}

#[test]
fn aborting_during_quota_wait_prevents_a_delayed_commit_from_publishing_data() {
    let fixture = Fixture::new();
    let client = fixture.database("aborted-owner");
    let original = client.committed().unwrap();
    let token = fixture.owner.reserve_commit();
    let completion = fixture.commit(&client);
    execute(&client, Command::Abort { transaction: TX });
    drop(token);
    let reply = receive(&completion).unwrap();
    assert!(matches!(
        reply.result,
        Err(IndexedDbError::TransactionInactive(_))
    ));
    assert_eq!(reply.active_transaction, None);
    assert_eq!(*client.committed().unwrap(), *original);
    // A later upgrade can reuse the version rolled back by owner retirement.
    execute(
        &client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(2),
            admission: None,
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
        CommandResult::Count(0)
    ));
    receive(&fixture.commit(&client)).unwrap().result.unwrap();
}

#[test]
fn a_waiting_bucket_does_not_block_another_buckets_sql_commit() {
    let first = Fixture::new();
    let store = &first.owner.store;
    let identity = store
        .lock()
        .open_bucket("test-storage-key", "independent")
        .unwrap();
    let second = Fixture {
        owner: storage_bucket_quota_owner(store, &identity.locator()).unwrap(),
        _manager: first._manager.clone(),
    };
    assert!(Arc::ptr_eq(
        &first.owner.storage_service,
        &second.owner.storage_service
    ));
    let blocked = first.database("waiting-for-quota");
    let independent = second.database("independent");
    let token = first.owner.reserve_commit();
    let waiting = first.commit(&blocked);
    let progressing = second.commit(&independent);
    receive(&progressing).unwrap().result.unwrap();
    assert_eq!(independent.committed().unwrap().version, 2);
    assert_eq!(blocked.committed().unwrap().version, 1);
    assert!(matches!(waiting.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(token);
    receive(&waiting).unwrap().result.unwrap();
    assert_eq!(blocked.committed().unwrap().version, 2);
}

#[test]
fn blocked_bucket_maintenance_cannot_block_ready_sql_quota_work() {
    let first = Fixture::new();
    let store = &first.owner.store;
    let identity = store
        .lock()
        .open_bucket("test-storage-key", "independent")
        .unwrap();
    let second = Fixture {
        owner: storage_bucket_quota_owner(store, &identity.locator()).unwrap(),
        _manager: first._manager.clone(),
    };
    let blocked = first.database("waiting-behind-maintenance");
    let independent = second.database("independent");
    let token = first.owner.reserve_commit();
    let coordinator = first.owner.storage_service.quota_coordinator.clone();
    let locator = first.owner.locator.clone();
    let (entered, waiting) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    first
        .owner
        .storage_service
        .maintenance_queue
        .dispatch(
            move || {
                entered.send(()).unwrap();
                let _quota = coordinator.reserve(&locator);
            },
            move |result| {
                done.send(result).unwrap();
            },
        )
        .unwrap();
    receive(&waiting);
    let first_reply = first.commit(&blocked);
    let second_reply = second.commit(&independent);
    receive(&second_reply).unwrap().result.unwrap();
    assert_eq!(independent.committed().unwrap().version, 2);
    assert!(matches!(
        first_reply.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(token);
    receive(&first_reply).unwrap().result.unwrap();
    receive(&completed).unwrap();
}

#[test]
fn shutdown_dispatches_a_waiting_quota_commit_before_closing_sql() {
    let fixture = Fixture::new();
    let client = fixture.database("shutdown-with-quota-waiter");
    let token = fixture.owner.reserve_commit();
    let reply = fixture.commit(&client);
    let service = fixture.owner.storage_service.clone();
    let manager = fixture._manager.clone();
    let (started, starting) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        started.send(()).unwrap();
        service.drain_maintenance().unwrap();
        manager.lock().shutdown_sqlite().unwrap();
        done.send(()).unwrap();
    });
    receive(&starting);
    // The accepted commit cannot be dispatched while this exact bucket's
    // reservation is held. Shutdown must keep SQL alive until it can be sent.
    assert!(finished.recv_timeout(Duration::from_millis(50)).is_err());
    drop(token);
    receive(&finished);
    receive(&reply).unwrap().result.unwrap();
    shutdown.join().unwrap();
    assert!(client.is_retired());
    assert_eq!(client.committed().unwrap().version, 2);
}

#[test]
fn maintenance_drain_rejects_a_quota_worker_before_waiting_on_its_own_reservation() {
    let fixture = Fixture::new();
    let service = fixture.owner.storage_service.clone();
    let draining = service.clone();
    let locator = fixture.owner.locator.clone();
    let (sender, receiver) = mpsc::channel();
    service
        .dispatch_quota_commit(
            locator,
            move |_quota| draining.drain_maintenance(),
            move |result| sender.send(result).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        receive(&receiver).unwrap(),
        Err(crate::StorageServiceDispatchError::ReentrantDrain)
    ));
}

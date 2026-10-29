use super::*;
use crate::read_budget::ReadBudget;

#[test]
fn metadata_snapshots_share_definitions_and_keep_old_versions_charged() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "shared-schema", Location::Memory);
    initialize(&client);
    let before = client.committed().unwrap();
    let original_bytes = service.metadata_bytes();
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    write(&client, b"new contents");
    let CommandResult::Metadata(after) =
        execute(&client, Command::Commit { transaction: TX }).unwrap()
    else {
        panic!("commit publishes its budgeted snapshot");
    };
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(&before.stores, &after.stores));
    assert!(after.usage > before.usage);
    assert!(service.metadata_bytes() > original_bytes);
    let observer = after.clone();
    with_database(&client, move |database| {
        assert!(Arc::ptr_eq(database.committed(), &observer))
    });
    drop(before);
    assert_eq!(service.metadata_bytes(), original_bytes);
    let lease = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &lease);
    let CommandResult::Metadata(unchanged) =
        execute(&client, Command::Commit { transaction: TX }).unwrap()
    else {
        panic!("readonly commit publishes the existing snapshot");
    };
    assert!(Arc::ptr_eq(&after, &unchanged));
    assert_eq!(service.metadata_bytes(), original_bytes);
}

#[test]
fn full_metadata_budget_allows_readonly_commit_but_rolls_back_a_failed_upgrade() {
    let mut settings = config();
    settings.metadata_bytes = 64 * 1024;
    let service = Service::new(settings).unwrap();
    let client = open(&service, "metadata-commit", Location::Memory);
    initialize(&client);
    let original = client.committed().unwrap();
    let mut held = service.metadata.reservation();
    held.charge((settings.metadata_bytes - service.metadata_bytes()) as u64)
        .unwrap();
    let lease = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &lease);
    execute(&client, Command::Commit { transaction: TX }).unwrap();
    execute(
        &client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(2),
            admission: None,
        },
    )
    .unwrap();
    execute(
        &client,
        Command::CreateStore {
            transaction: TX,
            name: "new".into(),
            options: Default::default(),
        },
    )
    .unwrap();
    let failure = reply(&client, Command::Commit { transaction: TX });
    assert!(matches!(failure.result, Err(IndexedDbError::Resource(_))));
    assert_eq!(failure.active_transaction, None);
    assert!(Arc::ptr_eq(&original, &client.committed().unwrap()));
    assert_eq!(service.metadata_bytes(), settings.metadata_bytes);
    drop(held);
    execute(
        &client,
        Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(2),
            admission: None,
        },
    )
    .unwrap();
    execute(
        &client,
        Command::CreateStore {
            transaction: TX,
            name: "new".into(),
            options: Default::default(),
        },
    )
    .unwrap();
    execute(&client, Command::Commit { transaction: TX }).unwrap();
    let committed = client.committed().unwrap();
    assert_eq!(committed.version, 2);
    assert!(committed.stores.contains_key("new"));
}

#[test]
fn metadata_capacity_is_shared_between_workers_and_follows_the_last_snapshot() {
    let mut settings = config();
    settings.executor.workers = 2;
    settings.metadata_bytes = 128 * 1024;
    let service = Service::new(settings).unwrap();
    let budget = service.metadata.clone();
    let mut clients = Vec::new();
    let mut rejected = false;
    for id in 0..64 {
        let (sender, receiver) = mpsc::channel();
        let client = match service.open(
            identity(&format!("db-{id}")),
            Location::Memory,
            move |reply| {
                let _ = sender.send(reply);
            },
        ) {
            Ok(client) => client,
            Err(DispatchError::MetadataCapacity) => {
                rejected = true;
                break;
            }
            Err(error) => panic!("unexpected registration admission: {error:?}"),
        };
        let result = receive(&receiver);
        assert!(service.metadata_bytes() <= settings.metadata_bytes);
        match result.result {
            Ok(_) => clients.push(client),
            Err(IndexedDbError::Resource(message)) => {
                assert!(message.contains("metadata"));
                rejected = true;
                break;
            }
            other => panic!("unexpected registration result: {other:?}"),
        }
    }
    assert!(rejected);
    assert!(clients.len() >= 2);
    let snapshot = clients[0].committed().unwrap();
    let definitions = snapshot.stores.clone();
    drop(service);
    drop(clients);
    assert!(budget.used() > 0);
    drop(snapshot);
    assert!(budget.used() > 0);
    drop(definitions);
    assert_eq!(budget.used(), 0);
}

#[test]
fn metadata_open_capacity_failure_can_retry_private_memory_and_prepared_disk() {
    let directory = Directory::new();
    for location in [Location::Memory, directory.location("db.sqlite")] {
        let service = Service::new(config()).unwrap();
        let gate = registration_tests::pause_worker(&service);
        let (sender, receiver) = mpsc::channel();
        let client = service
            .open(identity("retry"), location.clone(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        let mut held = service.metadata.reservation();
        held.charge((config().metadata_bytes - service.metadata_bytes()) as u64)
            .unwrap();
        drop(gate);
        let failure = receive(&receiver);
        assert!(matches!(failure.result, Err(IndexedDbError::Resource(_))));
        assert!(client.committed().is_none());
        drop(failure);
        drop(held);
        let (retried, result) = register(&service, "retry", location);
        assert!(matches!(result.result, Ok(CommandResult::Metadata(_))));
        assert_eq!(client.id(), retried.id());
        drop(result);
        initialize(&retried);
        assert_eq!(client.committed().unwrap().version, 1);
    }
}

#[test]
fn retained_metadata_versions_can_exhaust_capacity_and_release_it_for_the_next_commit() {
    let mut settings = config();
    settings.metadata_bytes = 64 * 1024;
    let service = Service::new(settings).unwrap();
    let client = open(&service, "retained", Location::Memory);
    initialize(&client);
    let mut retained = Vec::new();
    let mut rejected = false;
    for _ in 0..1000 {
        retained.push(client.committed().unwrap());
        let lease = queue(&client, "a", TransactionMode::ReadWrite);
        start(&client, "a", TransactionMode::ReadWrite, &lease);
        write(&client, b"record");
        match execute(&client, Command::Commit { transaction: TX }) {
            Ok(_) => {}
            Err(IndexedDbError::Resource(_)) => {
                rejected = true;
                break;
            }
            other => panic!("unexpected commit result: {other:?}"),
        }
    }
    assert!(rejected);
    assert!(service.metadata_bytes() <= settings.metadata_bytes);
    let successful = retained.len() as u64 - 1;
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    assert!(matches!(execute(&client, Command::Count {
        transaction: TX, store: "a".into(), source: Default::default(), range: Default::default(),
    }), Ok(CommandResult::Count(count)) if count == successful));
    execute(&client, Command::Commit { transaction: TX }).unwrap();
    drop(retained);
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    write(&client, b"after release");
    execute(&client, Command::Commit { transaction: TX }).unwrap();
    let reader = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &reader);
    assert!(matches!(execute(&client, Command::Count {
        transaction: TX, store: "a".into(), source: Default::default(), range: Default::default(),
    }), Ok(CommandResult::Count(count)) if count == successful + 1));
    execute(&client, Command::Commit { transaction: TX }).unwrap();
}

use super::*;
use moli_file_api::BlobData;

fn blob_value(bytes: Vec<u8>) -> crate::IndexedDbValue {
    crate::IndexedDbValue::new(
        vec![1],
        vec![crate::IndexedDbExternalObject::Blob {
            bytes: bytes.into(),
            mime_type: "application/octet-stream".into(),
        }],
    )
}

fn put_blob(client: &DatabaseClient, bytes: Vec<u8>) {
    execute(
        client,
        Command::Write {
            transaction: TX,
            store: "a".into(),
            key: Some(1.into()),
            value: blob_value(bytes),
            indexes: Vec::new(),
            add_only: false,
            quota: None,
        },
    )
    .unwrap();
}

fn read_blob(client: &DatabaseClient) -> BlobData {
    let CommandResult::Records(records) = execute(
        client,
        Command::Query {
            transaction: TX,
            store: "a".into(),
            query: Query {
                blob_references: true,
                ..Default::default()
            },
        },
    )
    .unwrap() else {
        panic!("Blob query");
    };
    let crate::IndexedDbExternalObject::Blob { bytes, .. } =
        &records[0].value.as_ref().unwrap().external_objects()[0]
    else {
        panic!("Blob attachment");
    };
    bytes.clone()
}

fn commit(client: &DatabaseClient) {
    execute(
        client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}

fn replace_chunk_for_test(client: &DatabaseClient, bytes: Vec<u8>) {
    with_database(client, move |database| {
        database
            .connection
            .execute("UPDATE blob_chunks SET bytes=?1", [bytes])
            .unwrap();
    });
}

#[test]
fn lazy_blob_preservation_error_rolls_back_delete_and_allows_the_reference_to_retry() {
    let mut service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "preservation-error");
    initialize(&client);
    let write = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &write);
    put_blob(&client, vec![7; 4096]);
    commit(&client);
    let read = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &read);
    let blob = read_blob(&client);
    commit(&client);
    replace_chunk_for_test(&client, vec![0]);
    let write = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &write);
    let deletion = execute(
        &client,
        Command::Delete {
            transaction: TX,
            store: "a".into(),
            range: Default::default(),
        },
    );
    assert!(
        matches!(deletion, Err(IndexedDbError::Corruption(_))),
        "a failed preservation cannot destroy the source: {deletion:?}"
    );
    assert!(matches!(
        execute(
            &client,
            Command::Count {
                transaction: TX,
                store: "a".into(),
                source: Default::default(),
                range: Default::default()
            }
        )
        .unwrap(),
        CommandResult::Count(1)
    ));
    replace_chunk_for_test(&client, vec![7; 4096]);
    assert_eq!(blob.read_blocking().unwrap(), vec![7; 4096]);
    execute(
        &client,
        Command::Delete {
            transaction: TX,
            store: "a".into(),
            range: Default::default(),
        },
    )
    .unwrap();
    commit(&client);
    service.shutdown().unwrap();
    assert_eq!(blob.read_blocking().unwrap(), vec![7; 4096]);
}

#[test]
fn lazy_sql_blob_slices_reopen_after_web_close_and_survive_service_shutdown() {
    let directory = Directory::new();
    let mut service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "lazy-body");
    let connection = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    initialize(&client);
    let write = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &write);
    let expected: Vec<_> = (0..CHUNK_BYTES * 3 + 17)
        .map(|index| (index % 251) as u8)
        .collect();
    put_blob(&client, expected.clone());
    commit(&client);
    let read = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &read);
    let blob = read_blob(&client);
    assert_eq!(blob.len(), expected.len());
    assert!(
        blob.as_bytes().is_none(),
        "query returned a source, not its body"
    );
    commit(&client);
    let (send, receive) = mpsc::channel();
    connection
        .close(move |reply| send.send(reply).unwrap())
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(physical_connection(&client));
    reap_idle(&service);
    assert!(!physical_connection(&client));
    let range = CHUNK_BYTES - 3..CHUNK_BYTES + 5;
    assert_eq!(
        blob.slice(range.clone()).read_blocking().unwrap(),
        expected[range]
    );
    assert!(
        physical_connection(&client),
        "Blob reads reuse the idle grace period"
    );
    reap_idle(&service);
    assert!(
        !physical_connection(&client),
        "a Blob does not pin the physical connection"
    );
    service.shutdown().unwrap();
    assert_eq!(blob.read_blocking().unwrap(), expected);
}

#[test]
fn lazy_blob_preserves_old_content_on_delete_and_owns_new_uncommitted_content_on_abort() {
    let mut service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "blob-rollback");
    initialize(&client);
    let write = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &write);
    put_blob(&client, b"old".to_vec());
    commit(&client);
    let read = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &read);
    let old = read_blob(&client);
    assert!(old.as_bytes().is_none());
    execute(
        &client,
        Command::Delete {
            transaction: TX,
            store: "a".into(),
            range: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(old.read_blocking().unwrap(), b"old");
    put_blob(&client, b"uncommitted".to_vec());
    let new = read_blob(&client);
    assert!(
        new.as_bytes().is_some(),
        "uncommitted object IDs cannot outlive their transaction"
    );
    execute(&client, Command::Abort { transaction: TX }).unwrap();
    assert_eq!(old.read_blocking().unwrap(), b"old");
    assert_eq!(new.read_blocking().unwrap(), b"uncommitted");
    let next = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &next);
    put_blob(&client, b"replacement".to_vec());
    commit(&client);
    service.shutdown().unwrap();
    assert_eq!(old.read_blocking().unwrap(), b"old");
    assert_eq!(new.read_blocking().unwrap(), b"uncommitted");
}

#[test]
fn lazy_blob_metadata_does_not_read_corrupt_body_and_read_reports_the_error() {
    let mut service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "blob-corruption");
    initialize(&client);
    let write = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &write);
    put_blob(&client, vec![7; 4096]);
    commit(&client);
    with_database(&client, |database| {
        database
            .connection
            .execute("UPDATE blob_chunks SET bytes=x'00'", [])
            .unwrap();
    });
    let read = queue(&client, "a", TransactionMode::ReadOnly);
    start(&client, "a", TransactionMode::ReadOnly, &read);
    let blob = read_blob(&client);
    assert_eq!(blob.len(), 4096);
    commit(&client);
    assert!(blob.read_blocking().is_err());
    service.shutdown().unwrap();
    assert!(blob.read_blocking().is_err());
}

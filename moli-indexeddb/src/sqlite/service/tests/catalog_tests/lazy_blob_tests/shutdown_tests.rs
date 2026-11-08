use super::*;

fn committed_blob(client: &DatabaseClient) -> BlobData {
    initialize(client);
    let write = queue(client, "a", TransactionMode::ReadWrite);
    start(client, "a", TransactionMode::ReadWrite, &write);
    put_blob(client, b"survives database retirement".to_vec());
    commit(client);
    let read = queue(client, "a", TransactionMode::ReadOnly);
    start(client, "a", TransactionMode::ReadOnly, &read);
    let blob = read_blob(client);
    commit(client);
    assert!(blob.as_bytes().is_none());
    blob
}

fn wait_for_stop(client: &DatabaseClient) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.owner.is_stopping() {
        assert!(Instant::now() < deadline, "owner mailbox was not sealed");
        std::thread::yield_now();
    }
}

fn read_async(blob: &BlobData) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    blob.read_all(Box::new(move |result| {
        let _ = sender.send(result);
    }));
    receiver
}

fn assert_pending(receiver: &mpsc::Receiver<std::io::Result<Vec<u8>>>) {
    assert!(
        matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "a read must wait for preservation while the stopped owner is blocked"
    );
}

fn read_result(receiver: &mpsc::Receiver<std::io::Result<Vec<u8>>>) -> std::io::Result<Vec<u8>> {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("Blob read must complete after the owner closes")
}

#[test]
fn lazy_blob_reads_wait_for_owner_clear_preservation() {
    let directory = Directory::new();
    let mut service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "blob-clear-race");
    let blob = committed_blob(&client);
    std::thread::scope(|scope| {
        let held = hold_database(&client);
        let clear = service.prepare_clear_owner(OWNER.to_vec()).unwrap();
        let clearing = scope.spawn(move || clear.execute());
        wait_for_stop(&client);
        let whole = read_async(&blob);
        let slice = read_async(&blob.slice(9..17));
        assert_pending(&whole);
        assert_pending(&slice);
        drop(held);
        assert_eq!(
            read_result(&whole).unwrap(),
            b"survives database retirement"
        );
        assert_eq!(read_result(&slice).unwrap(), b"database");
        clearing.join().unwrap().unwrap();
    });
    assert!(service.databases(OWNER).unwrap().is_empty());
    service.shutdown().unwrap();
    assert_eq!(
        blob.read_blocking().unwrap(),
        b"survives database retirement"
    );
}

#[test]
fn lazy_blob_reads_wait_for_service_shutdown_preservation() {
    let mut service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "blob-shutdown-race");
    let blob = committed_blob(&client);
    std::thread::scope(|scope| {
        let held = hold_database(&client);
        let shutdown = scope.spawn(|| service.shutdown());
        wait_for_stop(&client);
        let first = read_async(&blob);
        let second = read_async(&blob);
        assert_pending(&first);
        assert_pending(&second);
        drop(held);
        shutdown.join().unwrap().unwrap();
        assert_eq!(
            read_result(&first).unwrap(),
            b"survives database retirement"
        );
        assert_eq!(
            read_result(&second).unwrap(),
            b"survives database retirement"
        );
    });
    assert_eq!(
        blob.read_blocking().unwrap(),
        b"survives database retirement"
    );
}

#[test]
fn lazy_blob_reads_queued_after_database_delete_use_the_preserved_bytes() {
    let directory = Directory::new();
    let mut service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let client = resolve(&service, OWNER, "blob-delete-race");
    let blob = committed_blob(&client);
    let held = hold_database(&client);
    let (deleted, deletion) = mpsc::channel();
    client
        .submit(Command::DeleteDatabase, move |reply| {
            let _ = deleted.send(reply);
        })
        .unwrap();
    let reading = read_async(&blob);
    assert!(!client.owner.is_stopping());
    assert_pending(&reading);
    drop(held);
    receive(&deletion).result.unwrap();
    assert_eq!(
        read_result(&reading).unwrap(),
        b"survives database retirement"
    );
    service.shutdown().unwrap();
    assert_eq!(
        blob.read_blocking().unwrap(),
        b"survives database retirement"
    );
}

#[test]
fn lazy_blob_reads_wait_for_shutdown_preservation_errors() {
    let mut service = Service::new(config()).unwrap();
    let client = resolve(&service, OWNER, "blob-failed-preservation-race");
    let blob = committed_blob(&client);
    replace_chunk_for_test(&client, vec![0]);
    std::thread::scope(|scope| {
        let held = hold_database(&client);
        let shutdown = scope.spawn(|| service.shutdown());
        wait_for_stop(&client);
        let reading = read_async(&blob);
        assert_pending(&reading);
        drop(held);
        shutdown.join().unwrap().unwrap();
        let error = read_result(&reading).unwrap_err();
        assert_eq!(
            blob.read_blocking().unwrap_err().to_string(),
            error.to_string()
        );
    });
}

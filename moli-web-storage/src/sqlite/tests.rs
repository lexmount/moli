use super::*;
use moli_sqlite::Runtime;
use std::{sync::mpsc, thread, time::Duration};

fn open(root: &Path, runtime: &Runtime) -> (SharedWebStorageStore, SqliteWebStorageOwner) {
    new_sqlite_web_storage_store(
        root.join("localstorage.sqlite"),
        OpenMode::Create,
        runtime.handle(),
    )
    .unwrap()
}
fn backend(store: &WebStorageStore) -> &SqliteBackend {
    let WebStorageBackend::Sqlite(backend) = &store.backend else {
        panic!("SQLite fixture");
    };
    backend
}
fn resident(store: &WebStorageStore) -> usize {
    backend(store)
        .cache
        .lock()
        .areas
        .values()
        .filter(|area| area.strong_count() > 0)
        .count()
}

#[test]
fn utf16_blobs_empty_strings_and_partitioned_areas_survive_close_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let a = web_storage_partitioned_area_key("https://a.test", "https://top.test");
    let b = web_storage_partitioned_area_key("https://a.test", "https://other.test");
    let key = [0xd800, 0, 0xdc00];
    let value = [0xdc00, 0, 0xd800];
    store.try_set_item_utf16(&a, &key, &value).unwrap();
    store.try_set_item(&a, "", "").unwrap();
    store.try_set_item(&b, "private", "other").unwrap();
    owner.close().unwrap();
    assert!(store.try_get_item_utf16(&a, &key).is_err());
    drop(store);
    drop(owner);
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(resident(&store), 0);
    let _view = store.lease_area(&a).unwrap();
    assert_eq!(
        store.try_get_item_utf16(&a, &key).unwrap(),
        Some(value.to_vec())
    );
    assert_eq!(store.try_get_item_utf16(&a, &[]).unwrap(), Some(vec![]));
    assert_eq!(store.try_len(&a).unwrap(), 2);
    assert_eq!(store.try_get_item_utf16(&b, &key).unwrap(), None);
    assert_eq!(store.get_item(&b, "private").as_deref(), Some("other"));
    owner.close().unwrap();
    runtime.shutdown().unwrap();
}
#[test]
fn warmed_reads_and_writes_do_not_wait_for_the_database() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let key = "https://warm.test";
    let _view = store.lease_area(key).unwrap();
    store.try_set_item(key, "value", "before").unwrap();
    owner.flush().unwrap();
    let client = backend(&store).client.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocked = thread::spawn(move || {
        client.query(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    store
        .try_set_item(key, "value", "accepted while SQL is blocked")
        .unwrap();
    assert_eq!(
        store.get_item(key, "value").as_deref(),
        Some("accepted while SQL is blocked")
    );
    release_tx.send(()).unwrap();
    blocked.join().unwrap().unwrap();
    owner.flush().unwrap();
}
#[test]
fn asynchronous_failure_keeps_accepted_values_and_rejects_later_mutations_until_explicit_flush() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let key = "https://failure.test";
    let _view = store.lease_area(key).unwrap();
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    backend(&store).client.query(|connection| {connection.execute_batch("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(FAIL,'injected persistence failure'); END;")?;Ok(())}).unwrap();
    store.try_set_item(key, "key", "accepted").unwrap();
    assert!(owner.flush().is_err());
    assert_eq!(store.get_item(key, "key").as_deref(), Some("accepted"));
    assert_eq!(events.drain().len(), 1);
    assert!(matches!(
        store.try_set_item(key, "key", "rejected"),
        Err(WebStorageMutationError::Persistence(_))
    ));
    assert_eq!(store.get_item(key, "key").as_deref(), Some("accepted"));
    assert!(events.is_empty());
    backend(&store)
        .client
        .query(|connection| {
            connection.execute_batch("DROP TRIGGER fail_entries")?;
            Ok(())
        })
        .unwrap();
    owner.flush().unwrap();
    store.try_set_item(key, "key", "recovered").unwrap();
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.get_item(key, "key").as_deref(), Some("recovered"));
    owner.close().unwrap();
}
#[test]
fn clear_then_put_composes_across_areas_and_logical_events_remain_individual() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let _a = store.lease_area("a").unwrap();
    let _b = store.lease_area("b").unwrap();
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    store.try_set_item("a", "old", "gone").unwrap();
    store.try_set_item("b", "b", "separate").unwrap();
    owner.flush().unwrap();
    events.drain();
    store.try_set_item("a", "k", "one").unwrap();
    store.try_set_item("a", "k", "two").unwrap();
    store.try_clear("a").unwrap();
    store.try_set_item("a", "new", "kept").unwrap();
    assert_eq!(events.drain().len(), 4);
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.sorted_keys("a"), vec!["new"]);
    assert_eq!(store.get_item("a", "new").as_deref(), Some("kept"));
    assert_eq!(store.get_item("b", "b").as_deref(), Some("separate"));
    owner.close().unwrap();
}
#[test]
fn last_view_release_flushes_and_clean_areas_can_be_unloaded() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let view = store.lease_area("area").unwrap();
    store.try_set_item("area", "k", "v").unwrap();
    assert_eq!(resident(&store), 1);
    drop(view);
    // The last-view flush is ahead of this query in the database FIFO.
    assert_eq!(
        backend(&store)
            .client
            .query(|connection| Ok(connection.query_row(
                "SELECT count(*) FROM entries",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
    assert_eq!(resident(&store), 0);
    let _view = store.lease_area("area").unwrap();
    assert_eq!(store.get_item("area", "k").as_deref(), Some("v"));
    owner.close().unwrap();
}
#[test]
fn malformed_persisted_area_is_reported_instead_of_appearing_empty() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    store.try_set_item("area", "k", "v").unwrap();
    owner.flush().unwrap();
    assert_eq!(resident(&store), 0);
    backend(&store)
        .client
        .query(|connection| {
            connection.execute("UPDATE areas SET entry_count=99", [])?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        store.try_get_item_utf16("area", &[107]),
        Err(WebStorageMutationError::Persistence(_))
    ));
    owner.close().unwrap();
}
#[test]
fn quota_rejection_and_same_value_writes_do_not_publish_mutations() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let _view = store.lease_area("area").unwrap();
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    store.try_set_item("area", "k", "v").unwrap();
    events.drain();
    store.try_set_item("area", "k", "v").unwrap();
    assert_eq!(
        store.try_set_item("area", "k", &"x".repeat(WEB_STORAGE_QUOTA_BYTES + 1)),
        Err(WebStorageMutationError::QuotaExceeded)
    );
    assert_eq!(store.get_item("area", "k").as_deref(), Some("v"));
    assert!(events.is_empty());
    owner.close().unwrap();
}
#[test]
fn transient_keys_do_not_accumulate_delete_history_in_a_batch() {
    let pin = Arc::new(Area {
        state: Mutex::new(None),
        views: AtomicUsize::new(0),
    });
    let mut batch = Batch::default();
    for i in 0..10_000 {
        let key = WebStorageString::from_utf8(&i.to_string());
        for change in [
            KeyChange::Put {
                value: WebStorageString::from_utf8("v"),
                existed: false,
            },
            KeyChange::Delete,
        ] {
            let delta = AreaDelta {
                clear: false,
                changes: BTreeMap::from([(key.clone(), change)]),
                usage: 0,
                count: 0,
                _pin: pin.clone(),
            };
            batch.merge(Batch(BTreeMap::from([("area".into(), delta)])));
        }
    }
    assert!(batch.0["area"].changes.is_empty());
}

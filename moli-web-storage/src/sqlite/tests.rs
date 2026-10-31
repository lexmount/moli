use super::*;
use moli_sqlite::Runtime;
use std::{sync::mpsc, thread, time::Duration};

#[test]
fn explicit_import_rolls_back_if_a_later_destination_area_is_corrupt() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("localstorage.sqlite");
    let runtime = Runtime::new()?;
    let (store, owner) = open(root.path(), &runtime);
    for area in ["a", "z"] {
        store.try_set_item(area, "key", "old")?;
    }
    owner.close()?;
    {
        let connection = Connection::open(&path)?;
        connection.execute("UPDATE areas SET usage_bytes=999 WHERE storage_key='z'", [])?;
    }
    let areas = ["a", "z"]
        .into_iter()
        .map(|area| {
            (
                area.to_owned(),
                vec![(
                    "key".encode_utf16().collect(),
                    "new".encode_utf16().collect(),
                )],
            )
        })
        .collect();
    assert!(import_sqlite_web_storage(path, runtime.handle(), areas).is_err());
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(
        store.try_get_item_utf16("a", &"key".encode_utf16().collect::<Vec<_>>())?,
        Some("old".encode_utf16().collect())
    );
    owner.close()?;
    Ok(())
}

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
fn acquiring_an_unused_store_and_area_lease_does_not_open_the_database() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("localstorage.sqlite");
    let runtime = Runtime::new().unwrap();
    let (store, owner) =
        new_lazy_sqlite_web_storage_store(path.clone(), OpenMode::Create, runtime.handle())
            .unwrap();
    let lease = store.lease_area("https://unused.test").unwrap();
    drop(lease);
    owner.flush().unwrap();
    owner.close().unwrap();
    assert!(root.path().read_dir().unwrap().next().is_none());
    assert!(store.try_len("https://unused.test").is_err());
    assert!(root.path().read_dir().unwrap().next().is_none());
}

#[test]
fn the_first_local_storage_read_restores_persisted_values() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    store
        .try_set_item("https://restore.test", "key", "saved")
        .unwrap();
    owner.close().unwrap();
    let (store, owner) = new_lazy_sqlite_web_storage_store(
        root.path().join("localstorage.sqlite"),
        OpenMode::Create,
        runtime.handle(),
    )
    .unwrap();
    assert_eq!(resident(&store), 0);
    assert_eq!(
        store
            .try_get_item_utf16(
                "https://restore.test",
                &"key".encode_utf16().collect::<Vec<_>>()
            )
            .unwrap(),
        Some("saved".encode_utf16().collect())
    );
    store
        .try_set_item("https://restore.test", "key", "changed")
        .unwrap();
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(
        store.get_item("https://restore.test", "key").as_deref(),
        Some("changed")
    );
    owner.close().unwrap();
}

#[test]
fn local_storage_open_failure_is_reported_on_first_access() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("not-a-directory");
    std::fs::write(&parent, b"keep").unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = new_lazy_sqlite_web_storage_store(
        parent.join("localstorage.sqlite"),
        OpenMode::Create,
        runtime.handle(),
    )
    .unwrap();
    owner.flush().unwrap();
    assert!(store.try_len("https://failure.test").is_err());
    assert!(
        store
            .try_set_item("https://failure.test", "key", "value")
            .is_err()
    );
    assert!(owner.flush().is_err());
    assert!(owner.close().is_err());
    assert_eq!(std::fs::read(parent).unwrap(), b"keep");
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

#[test]
fn sqlite_full_keeps_dirty_data_and_explicit_flush_recovers_after_capacity_is_restored() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let _view = store.lease_area("area").unwrap();
    backend(&store)
        .client
        .query(|connection| {
            let pages: i64 = connection.query_row("PRAGMA page_count", [], |r| r.get(0))?;
            connection.pragma_update(None, "max_page_count", pages)?;
            Ok(())
        })
        .unwrap();
    let value = "x".repeat(1024 * 1024);
    store.try_set_item("area", "accepted", &value).unwrap();
    assert!(owner.flush().is_err());
    assert_eq!(store.get_item("area", "accepted"), Some(value.clone()));
    backend(&store)
        .client
        .query(|connection| {
            connection.pragma_update(None, "max_page_count", 1_000_000)?;
            Ok(())
        })
        .unwrap();
    owner.flush().unwrap();
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.get_item("area", "accepted"), Some(value));
    owner.close().unwrap();
}
#[test]
fn origin_clear_includes_cold_partitioned_areas_and_preserves_other_origins() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let a = web_storage_partitioned_area_key("https://same.test", "https://top.test");
    let b = web_storage_partitioned_area_key("https://same.test", "https://other.test");
    let other = web_storage_partitioned_area_key("https://other.test", "https://other.test");
    for key in [&a, &b, &other] {
        store.try_set_item(key, "value", "persisted").unwrap();
    }
    owner.flush().unwrap();
    assert_eq!(resident(&store), 0);
    let _view = store.lease_area(&a).unwrap();
    assert_eq!(store.get_item(&a, "value").as_deref(), Some("persisted"));
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    store.try_clear_origin_areas("https://same.test").unwrap();
    assert_eq!(events.drain().len(), 2);
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.try_len(&a).unwrap(), 0);
    assert_eq!(store.try_len(&b).unwrap(), 0);
    assert_eq!(store.try_len(&other).unwrap(), 1);
    owner.close().unwrap();
}

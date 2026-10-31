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
fn sqlite_quota_counts_keys_and_values_for_insert_replace_remove_and_clear() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let _view = store.lease_area("https://key-quota.test").unwrap();
    crate::tests::assert_key_and_value_quota(&store);
    owner.close().unwrap();
}

#[test]
fn imported_usage_includes_keys_and_survives_reopen_and_edits() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("localstorage.sqlite");
    let runtime = Runtime::new()?;
    let area = "https://import-quota.test";
    let unicode_key = vec![0xd800, 0, 0x0100];
    import_sqlite_web_storage(
        path.clone(),
        runtime.handle(),
        BTreeMap::from([(
            area.to_owned(),
            vec![
                (utf16_units("ascii"), vec![]),
                (unicode_key.clone(), vec![0xdc00, 0xd83d, 0xde00]),
                (utf16_units("ascii"), utf16_units("old")),
                (utf16_units("ascii"), vec![]),
            ],
        )]),
    )?;
    {
        let connection = Connection::open(&path)?;
        let metadata: (i64, i64, i64) = connection.query_row(
            "SELECT usage_bytes,entry_count,(SELECT SUM(length(key)+length(value)) FROM entries WHERE entries.area_id=areas.area_id) FROM areas WHERE storage_key=?1",
            [area],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(metadata, (22, 2, 22));
    }
    let (store, owner) = open(root.path(), &runtime);
    let view = store.lease_area(area)?.unwrap();
    assert_eq!(store.try_usage_bytes(area)?, 22);
    assert!(
        view.area.state.lock().is_none(),
        "usage remains metadata-only"
    );
    assert_eq!(store.try_len(area)?, 2);
    store.try_set_item(area, "ascii", "x")?;
    assert_eq!(store.try_usage_bytes(area)?, 24);
    store.try_remove_item_utf16(area, &unicode_key)?;
    assert_eq!(store.try_usage_bytes(area)?, 12);
    owner.close()?;
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.try_usage_bytes(area)?, 12);
    assert_eq!(store.try_len(area)?, 1);
    assert_eq!(store.get_item(area, "ascii").as_deref(), Some("x"));
    store.try_remove_item(area, "ascii")?;
    assert_eq!(store.try_usage_bytes(area)?, 0);
    owner.close()?;
    Ok(())
}

#[test]
fn restored_overquota_import_allows_shrinking_but_rejects_growth() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("localstorage.sqlite");
    let runtime = Runtime::new()?;
    let area = "https://overquota-import.test";
    // Explicit imports preserve existing data, even above the Web API quota.
    let value = vec![u16::from(b'x'); WEB_STORAGE_QUOTA_BYTES / 2 + 8];
    import_sqlite_web_storage(
        path,
        runtime.handle(),
        BTreeMap::from([(area.to_owned(), vec![(utf16_units("key"), value.clone())])]),
    )?;
    let (store, owner) = open(root.path(), &runtime);
    let view = store.lease_area(area)?.unwrap();
    assert_eq!(store.try_usage_bytes(area)?, WEB_STORAGE_QUOTA_BYTES + 22);
    assert!(view.area.state.lock().is_none());
    let key = utf16_units("key");
    store.try_set_item_utf16(area, &key, &value[..value.len() - 1])?;
    assert_eq!(store.try_usage_bytes(area)?, WEB_STORAGE_QUOTA_BYTES + 20);
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    assert_eq!(
        store.try_set_item_utf16(area, &key, &value),
        Err(WebStorageMutationError::QuotaExceeded)
    );
    assert_eq!(
        store.try_set_item(area, "new-key", ""),
        Err(WebStorageMutationError::QuotaExceeded)
    );
    assert!(events.is_empty());
    store.try_remove_item(area, "key")?;
    assert_eq!(store.try_usage_bytes(area)?, 0);
    owner.close()?;
    Ok(())
}

#[test]
fn value_only_metadata_version_is_rejected_without_modifying_data() -> Result<()> {
    let mut connection = Connection::open_in_memory()?;
    initialize(&mut connection)?;
    connection.execute(
        "INSERT INTO areas(storage_key,usage_bytes,entry_count) VALUES ('old',0,1)",
        [],
    )?;
    connection.execute(
        "INSERT INTO entries(area_id,key,value) VALUES (last_insert_rowid(),?1,?2)",
        params![encode(&utf16_units("key")), encode(&[])],
    )?;
    connection.pragma_update(None, "user_version", 1)?;
    assert!(initialize(&mut connection).is_err());
    let row: (i64, i64, i64) = connection.query_row(
        "SELECT usage_bytes,entry_count,length(key) FROM areas JOIN entries USING(area_id)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(row, (0, 1, 6));
    assert_eq!(
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))?,
        1
    );
    Ok(())
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
fn retrying_committed_then_deleted_or_cleared_keys_is_idempotent() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection).unwrap();
    let pin = Arc::new(Area {
        state: Mutex::new(None),
        views: AtomicUsize::new(0),
    });
    let delta = |clear, changes, usage, count| {
        Batch(BTreeMap::from([(
            "area".to_owned(),
            AreaDelta {
                clear,
                changes,
                usage,
                count,
                _pin: pin.clone(),
            },
        )]))
    };
    let key = WebStorageString::from_utf8;
    let mut batch = delta(
        false,
        BTreeMap::from([
            (key("keep"), KeyChange::Put(key("v"))),
            (key("remove"), KeyChange::Put(key("old"))),
        ]),
        28,
        2,
    );
    // SQL committed, but publication was lost: replay must still apply the
    // later delete even though its original put created a previously new row.
    batch.write(&mut connection).unwrap();
    batch.merge(delta(
        false,
        BTreeMap::from([(key("remove"), KeyChange::Delete)]),
        10,
        1,
    ));
    batch.write(&mut connection).unwrap();
    batch.write(&mut connection).unwrap();
    let area = read_area(&connection, "area").unwrap();
    assert_eq!(area.values, HashMap::from([(key("keep"), key("v"))]));

    batch.merge(delta(true, BTreeMap::new(), 0, 0));
    batch.merge(delta(
        false,
        BTreeMap::from([(key("new"), KeyChange::Put(key("latest")))]),
        18,
        1,
    ));
    batch.write(&mut connection).unwrap();
    batch.write(&mut connection).unwrap();
    let area = read_area(&connection, "area").unwrap();
    assert_eq!(area.values, HashMap::from([(key("new"), key("latest"))]));
}

#[test]
fn sqlite_full_retains_dirty_data_for_explicit_flush_after_capacity_is_restored() {
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
    assert!(matches!(
        store.try_set_item("area", "later", "blocked"),
        Err(WebStorageMutationError::Persistence(_))
    ));
    backend(&store)
        .client
        .query(|connection| {
            connection.pragma_update(None, "max_page_count", 1_000_000)?;
            Ok(())
        })
        .unwrap();
    assert!(backend(&store).client.failure().is_some());
    owner.flush().unwrap();
    assert_eq!(
        backend(&store)
            .client
            .query(|connection| {
                Ok(
                    connection.query_row("SELECT length(value) FROM entries", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .unwrap(),
        i64::try_from(value.len() * 2).unwrap()
    );
    store.try_set_item("area", "later", "recovered").unwrap();
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.get_item("area", "accepted"), Some(value));
    assert_eq!(
        store.get_item("area", "later").as_deref(),
        Some("recovered")
    );
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

#[test]
fn single_area_usage_reads_cold_metadata_and_current_dirty_size() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let key = web_storage_partitioned_area_key("https://usage.test", "https://usage.test");
    store.try_set_item(&key, "key", "old").unwrap();
    owner.flush().unwrap();
    assert_eq!(resident(&store), 0);
    let cold = store.lease_area(&key).unwrap().unwrap();
    assert_eq!(store.try_usage_bytes(&key).unwrap(), 12);
    assert!(
        cold.area.state.lock().is_none(),
        "usage must not load entries"
    );
    store.try_set_item(&key, "key", "changed").unwrap();
    assert_eq!(store.try_usage_bytes(&key).unwrap(), 20);
    store.try_clear(&key).unwrap();
    assert_eq!(store.try_usage_bytes(&key).unwrap(), 0);
    drop(cold);
    owner.close().unwrap();
}

#[test]
fn origin_metadata_query_uses_bounded_index_searches() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let origin = "https://index.test";
    let (prefix, upper) = origin_prefix_range(origin);
    let plan = backend(&store)
        .client
        .query(move |connection| {
            let sql = format!("EXPLAIN QUERY PLAN {ORIGIN_METADATA_SQL}");
            let mut statement = connection.prepare(&sql)?;
            Ok(statement
                .query_map(params![origin, prefix, upper], |row| {
                    row.get::<_, String>(3)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .unwrap();
    assert!(
        plan.iter().any(|step| step.contains("storage_key=?")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step.contains("storage_key>? AND storage_key<?")),
        "{plan:?}"
    );
    assert!(
        plan.iter().all(|step| !step.contains("SCAN areas")),
        "{plan:?}"
    );
    owner.close().unwrap();
}

#[test]
fn origin_clear_deletes_cold_entries_without_loading_and_preserves_later_puts() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let origin = "https://clear-cold.test";
    let first = web_storage_partitioned_area_key(origin, origin);
    let other = web_storage_partitioned_area_key(origin, "https://other.test");
    let unrelated = web_storage_partitioned_area_key("https://clear-cold.test.sibling", origin);
    store.try_set_item(origin, "", "").unwrap();
    for key in [&first, &other, &unrelated] {
        store
            .try_set_item(key, "old", &"x".repeat(128 * 1024))
            .unwrap();
    }
    owner.flush().unwrap();
    assert_eq!(resident(&store), 0);
    let cold = store.lease_area(&first).unwrap().unwrap();
    assert!(cold.area.state.lock().is_none());
    let key = first.clone();
    backend(&store).client.query(move |connection| {
        // Reading entries would now fail metadata validation. Deletion must
        // depend only on map metadata, even for a damaged cold value.
        connection.execute("UPDATE entries SET value=X'0000' WHERE area_id=(SELECT area_id FROM areas WHERE storage_key=?1)", [key])?;
        Ok(())
    }).unwrap();
    let events = store.subscribe_mutations(WebStorageAreaKind::Local);
    assert!(store.try_clear_origin_areas(origin).unwrap());
    assert_eq!(
        events.drain().len(),
        3,
        "zero-byte nonempty maps also clear"
    );
    for key in [origin, &first, &other] {
        assert_eq!(
            store.try_len(key).unwrap(),
            0,
            "reads cannot restore old SQL data"
        );
    }
    assert!(!store.try_clear_origin_areas(origin).unwrap());
    assert!(events.is_empty());
    store.try_set_item(&first, "new", "kept").unwrap();
    drop(cold);
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.sorted_keys(&first), ["new"]);
    assert_eq!(store.get_item(&first, "new").as_deref(), Some("kept"));
    assert_eq!(store.try_len(&other).unwrap(), 0);
    assert_eq!(store.try_len(origin).unwrap(), 0);
    assert_eq!(store.try_len(&unrelated).unwrap(), 1);
    owner.close().unwrap();
}

#[test]
fn origin_clear_orders_after_accepted_writes_without_an_old_batch_resurrecting_them() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let origin = "https://clear-race.test";
    let key = web_storage_partitioned_area_key(origin, origin);
    store.try_set_item(&key, "old", "disk").unwrap();
    owner.flush().unwrap();
    store.try_set_item(&key, "old", "pending").unwrap();
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
    backend(&store).client.request_flush();
    let clearing = store.clone();
    let clear = thread::spawn(move || clearing.try_clear_origin_areas(origin));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while backend(&store).namespace.try_read().is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "clear never acquired namespace fence"
        );
        thread::sleep(Duration::from_millis(1));
    }
    release_tx.send(()).unwrap();
    blocked.join().unwrap().unwrap();
    assert!(clear.join().unwrap().unwrap());
    assert_eq!(store.try_len(&key).unwrap(), 0);
    store.try_set_item(&key, "new", "kept").unwrap();
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.sorted_keys(&key), ["new"]);
    assert_eq!(store.get_item(&key, "new").as_deref(), Some("kept"));
    owner.close().unwrap();
}

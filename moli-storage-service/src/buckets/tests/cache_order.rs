use super::super::StorageBucketCacheName;
use super::*;

const ORIGIN: &str = "https://cache-order.test";
const URL: &str = "https://cache-order.test/item";

fn response(body: &[u8]) -> StorageBucketCachedResponse {
    StorageBucketCachedResponse {
        cors_exposed_header_names: None,
        response_type: "default".to_owned(),
        url: URL.to_owned(),
        redirected: false,
        status: 200,
        status_text: "OK".to_owned(),
        headers: Vec::new(),
        body: body.to_vec(),
    }
}

#[test]
fn cache_creation_order_survives_reopen_delete_and_lossless_names() -> Result<()> {
    let temp = TempStorePath::new("cache-creation-order");
    let root = temp.cache_root();
    let names = vec![
        "z".into(),
        "a".into(),
        "Z".into(),
        "".into(),
        StorageBucketCacheName::from_utf16(vec![0xd800]),
        "\u{fffd}".into(),
        "\0".into(),
        "😀".into(),
    ];
    let expected = names
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, name)| name.clone())
        .chain([names[1].clone()])
        .collect::<Vec<_>>();
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        let mut handles = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let handle = store
                .open_cache_handle_for_identity(&identity, name)?
                .unwrap();
            handles.push(handle);
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &identity,
                    name,
                    URL,
                    response(&[index as u8]),
                    1,
                    0
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }
        assert_eq!(
            store.cache_names_for_identity(&identity),
            Some(names.clone())
        );
        assert_eq!(
            store.open_cache_handle_for_identity(&identity, &names[2])?,
            Some(handles[2])
        );
        assert_eq!(
            store.cache_names_for_identity(&identity),
            Some(names.clone())
        );
        assert_eq!(
            store.delete_cache_for_identity(&identity, &names[1])?,
            Some(true)
        );
        let recreated = store
            .open_cache_handle_for_identity(&identity, &names[1])?
            .unwrap();
        assert_ne!(recreated, handles[1]);
        assert_eq!(
            store.cache_entries_for_handle_and_identity(&identity, &names[1], recreated),
            Some(Vec::new())
        );
        assert_eq!(
            store
                .cache_entries_for_handle_and_identity(&identity, &names[1], handles[1])
                .unwrap()[0]
                .response
                .body,
            [1]
        );
        assert_eq!(
            store.cache_names_for_identity(&identity),
            Some(expected.clone())
        );
    }
    let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = store.lock();
    let identity = store.open_bucket(ORIGIN, "bucket")?;
    assert_eq!(store.cache_names_for_identity(&identity), Some(expected));
    for (index, name) in names.iter().enumerate() {
        let matched = store
            .match_cache_entry_for_identity(&identity, name, URL)
            .unwrap();
        if index == 1 {
            assert_eq!(matched, None);
        } else {
            assert_eq!(matched.unwrap().body, [index as u8]);
        }
    }
    Ok(())
}

#[test]
fn legacy_cache_files_keep_deterministic_order_and_gain_creation_order_on_save() -> Result<()> {
    let temp = TempStorePath::new("cache-legacy-order");
    let root = temp.cache_root();
    let bucket_dir;
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        bucket_dir = storage_bucket_cache_bucket_dir(&root, ORIGIN, identity.bucket_id());
        for name in ["z", "a"] {
            assert!(store.open_cache_for_identity(&identity, name)?);
            store.put_cache_entry_for_identity(
                &identity,
                name,
                URL,
                response(name.as_bytes()),
                1,
                0,
            )?;
        }
    }
    for entry in fs::read_dir(bucket_dir)? {
        let path = entry?.path();
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert!(
            value
                .as_object_mut()
                .unwrap()
                .remove("cacheOrder")
                .is_some()
        );
        fs::write(path, serde_json::to_vec(&value)?)?;
    }
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        assert_eq!(
            store.cache_names_for_identity(&identity),
            Some(vec!["a".into(), "z".into()])
        );
        for name in ["a", "z"] {
            assert_eq!(
                store
                    .match_cache_entry_for_identity(&identity, name, URL)
                    .flatten()
                    .unwrap()
                    .body,
                name.as_bytes()
            );
        }
        assert!(store.open_cache_for_identity(&identity, "Z")?);
    }
    let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = store.lock();
    let identity = store.open_bucket(ORIGIN, "bucket")?;
    assert_eq!(
        store.cache_names_for_identity(&identity),
        Some(vec!["a".into(), "z".into(), "Z".into()])
    );
    Ok(())
}

#[test]
fn cache_order_and_contents_recover_from_the_same_committed_root() -> Result<()> {
    for point in [
        CrashPoint::CacheNextDurable,
        CrashPoint::CachePreviousDurable,
        CrashPoint::CacheCurrentDurable,
        CrashPoint::CachePreviousRemovedBeforeSync,
    ] {
        let temp = TempStorePath::new("cache-order-crash");
        let root = temp.cache_root();
        {
            let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
            let mut store = store.lock();
            let identity = store.open_bucket(ORIGIN, "bucket")?;
            for name in ["z", "a"] {
                assert!(store.open_cache_for_identity(&identity, name)?);
                store.put_cache_entry_for_identity(
                    &identity,
                    name,
                    URL,
                    response(name.as_bytes()),
                    1,
                    0,
                )?;
            }
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    let _armed = arm(point);
                    store.open_cache_for_identity(&identity, "Z").unwrap();
                }))
                .is_err()
            );
        }
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        let expected = if matches!(point, CrashPoint::CacheNextDurable) {
            vec!["z".into(), "a".into()]
        } else {
            vec!["z".into(), "a".into(), "Z".into()]
        };
        assert_eq!(store.cache_names_for_identity(&identity), Some(expected));
        for name in ["z", "a"] {
            assert_eq!(
                store
                    .match_cache_entry_for_identity(&identity, name, URL)
                    .flatten()
                    .unwrap()
                    .body,
                name.as_bytes()
            );
        }
        let (next, previous) = storage_bucket_cache_replacement_paths(&root)?;
        assert!(root.exists());
        assert!(!next.exists());
        assert!(!previous.exists());
    }
    Ok(())
}

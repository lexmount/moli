use super::super::StorageBucketCachedRequestMetadata;
use super::*;

const ORIGIN: &str = "https://cache-request-state.test";
const URL: &str = "https://cache-request-state.test/navigation";

fn request(reload: bool) -> StorageBucketCachedRequest {
    StorageBucketCachedRequest {
        method: "GET".to_owned(),
        headers: vec![("cookie".to_owned(), "session=original".to_owned())],
        metadata: StorageBucketCachedRequestMetadata {
            destination: "document".to_owned(),
            referrer: "https://other.test/source".to_owned(),
            referrer_policy: "strict-origin".to_owned(),
            mode: "navigate".to_owned(),
            credentials: "include".to_owned(),
            cache: "no-store".to_owned(),
            redirect: "manual".to_owned(),
            integrity: "sha256-YWJj".to_owned(),
            keepalive: true,
            priority: "high".to_owned(),
            duplex: "half".to_owned(),
            is_history_navigation: !reload,
            is_reload_navigation: reload,
        },
    }
}

fn response(body: &[u8]) -> StorageBucketCachedResponse {
    StorageBucketCachedResponse {
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
fn cache_request_metadata_survives_profile_reopen_with_independent_contents() -> Result<()> {
    let temp = TempStorePath::new("cache-request-metadata");
    let root = temp.cache_root();
    let requests = [request(false), request(true)];
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        for (index, request) in requests.iter().enumerate() {
            assert_eq!(
                store.put_cache_entry_with_request_for_identity(
                    &identity,
                    "cache",
                    &format!("{URL}?index={index}"),
                    request.clone(),
                    response(&[index as u8]),
                    request.metadata.estimated_size_bytes() + 1,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }
    }
    let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = store.lock();
    let identity = store.open_bucket(ORIGIN, "bucket")?;
    let entries = store
        .cache_entries_for_identity(&identity, "cache")
        .unwrap();
    assert_eq!(entries.len(), 2);
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry.request_url, format!("{URL}?index={index}"));
        assert_eq!(entry.request, requests[index]);
        assert_eq!(entry.response.body, [index as u8]);
    }
    let mut local = entries[0].request.clone();
    local.metadata.mode = "cors".to_owned();
    local.headers.clear();
    assert_eq!(
        store
            .cache_entries_for_identity(&identity, "cache")
            .unwrap()[0]
            .request,
        requests[0]
    );
    Ok(())
}

#[test]
fn legacy_cache_request_metadata_defaults_without_losing_headers_or_body() -> Result<()> {
    let temp = TempStorePath::new("cache-request-metadata-legacy");
    let root = temp.cache_root();
    let bucket_dir;
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        bucket_dir = storage_bucket_cache_bucket_dir(&root, ORIGIN, identity.bucket_id());
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        store.put_cache_entry_with_request_for_identity(
            &identity,
            "cache",
            URL,
            request(true),
            response(b"legacy"),
            7,
            0,
        )?;
    }
    for entry in fs::read_dir(&bucket_dir)? {
        let path = entry?.path();
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["version"], 2);
        let record = value["entries"][0].as_object_mut().unwrap();
        assert!(record.remove("requestMetadata").is_some());
        fs::write(path, serde_json::to_vec(&value)?)?;
    }
    let expected = StorageBucketCachedRequest {
        metadata: StorageBucketCachedRequestMetadata::default(),
        ..request(true)
    };
    {
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        let entries = store
            .cache_entries_for_identity(&identity, "cache")
            .unwrap();
        assert_eq!(entries[0].request, expected);
        assert_eq!(entries[0].response.body, b"legacy");
        store.put_cache_entry_with_request_for_identity(
            &identity,
            "cache",
            &format!("{URL}?new"),
            request(false),
            response(b"new"),
            3,
            0,
        )?;
    }
    for entry in fs::read_dir(&bucket_dir)? {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(entry?.path())?)?;
        let records = value["entries"].as_array().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].get("requestMetadata").is_none());
        assert!(records[1].get("requestMetadata").is_some());
    }
    let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = store.lock();
    let identity = store.open_bucket(ORIGIN, "bucket")?;
    let entries = store
        .cache_entries_for_identity(&identity, "cache")
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].request, expected);
    assert_eq!(entries[0].response.body, b"legacy");
    assert_eq!(entries[1].request, request(false));
    assert_eq!(entries[1].response.body, b"new");
    Ok(())
}

#[test]
fn cache_request_metadata_and_response_recover_as_one_atomic_entry() -> Result<()> {
    for point in [
        CrashPoint::CacheNextDurable,
        CrashPoint::CachePreviousDurable,
        CrashPoint::CacheCurrentDurable,
        CrashPoint::CachePreviousRemovedBeforeSync,
    ] {
        let temp = TempStorePath::new("cache-request-metadata-crash");
        let root = temp.cache_root();
        {
            let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
            let mut store = store.lock();
            let identity = store.open_bucket(ORIGIN, "bucket")?;
            assert!(store.open_cache_for_identity(&identity, "cache")?);
            store.put_cache_entry_with_request_for_identity(
                &identity,
                "cache",
                URL,
                request(false),
                response(b"old"),
                3,
                0,
            )?;
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    let _armed = arm(point);
                    store
                        .put_cache_entry_with_request_for_identity(
                            &identity,
                            "cache",
                            URL,
                            request(true),
                            response(b"new"),
                            4,
                            0,
                        )
                        .unwrap();
                }))
                .is_err()
            );
        }
        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(ORIGIN, "bucket")?;
        let entries = store
            .cache_entries_for_identity(&identity, "cache")
            .unwrap();
        assert_eq!(entries.len(), 1);
        let old = matches!(point, CrashPoint::CacheNextDurable);
        assert_eq!(entries[0].request, request(!old));
        assert_eq!(entries[0].response.body, if old { b"old" } else { b"new" });
        assert_eq!(
            store.cache_usage_for_identity(&identity),
            Some(if old { 3 } else { 4 })
        );
        let (next, previous) = storage_bucket_cache_replacement_paths(&root)?;
        assert!(root.exists());
        assert!(!next.exists());
        assert!(!previous.exists());
    }
    Ok(())
}

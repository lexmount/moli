use super::super::{StorageBucketCachePut, load_storage_bucket_cache_file};
use super::*;

fn operation(
    url: &str,
    shape: &str,
    vary: Option<&str>,
    body: &str,
    usage_bytes: u64,
) -> StorageBucketCachePut {
    StorageBucketCachePut {
        request_url: url.to_owned(),
        request: StorageBucketCachedRequest {
            method: "GET".to_owned(),
            headers: vec![
                ("x-shape".to_owned(), shape.to_owned()),
                ("x-size".to_owned(), "big".to_owned()),
            ],
            ..StorageBucketCachedRequest::default()
        },
        response: StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: url.to_owned(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vary
                .map(|value| vec![("vary".to_owned(), value.as_bytes().to_vec())])
                .unwrap_or_default(),
            body: body.as_bytes().to_vec(),
        },
        usage_bytes,
    }
}

#[test]
fn cache_batch_preserves_vary_variants_replacement_order_and_selective_delete() -> Result<()> {
    let mut store = StorageBucketRegistry::default();
    let identity = store.open_bucket("https://batch.test", "bucket")?;
    let id = store
        .open_cache_handle_for_identity(&identity, "cache")?
        .unwrap();
    assert_eq!(
        store.put_cache_batch_for_handle_and_identity(
            &identity,
            "cache",
            id,
            vec![
                operation(
                    "https://batch.test/item#circle",
                    "circle",
                    Some("x-shape"),
                    "circle",
                    20
                ),
                operation(
                    "https://batch.test/item#square",
                    "square",
                    Some("x-shape"),
                    "square",
                    30
                ),
            ],
            0
        )?,
        StorageBucketCachePutOutcome::Stored
    );
    let query = |shape: &str| StorageBucketCacheQuery {
        request_url: "https://batch.test/item#query".to_owned(),
        method: "GET".to_owned(),
        headers: vec![("x-shape".to_owned(), shape.to_owned())],
        ignore_search: false,
        ignore_method: false,
        ignore_vary: false,
    };
    for shape in ["circle", "square"] {
        let matches = store
            .match_cache_entries_for_handle_and_identity(&identity, "cache", id, &query(shape))
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].response.body, shape.as_bytes());
    }
    let replacement = operation(
        "https://batch.test/item#new",
        "circle",
        Some("x-shape"),
        "updated",
        10,
    );
    assert_eq!(
        store.put_cache_entry_with_request_for_handle_and_identity(
            &identity,
            "cache",
            id,
            &replacement.request_url,
            replacement.request,
            replacement.response,
            replacement.usage_bytes,
            0
        )?,
        StorageBucketCachePutOutcome::Stored
    );
    let entries = store
        .cache_entries_for_handle_and_identity(&identity, "cache", id)
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.response.body.clone())
            .collect::<Vec<_>>(),
        vec![b"square".to_vec(), b"updated".to_vec()]
    );
    assert_eq!(store.cache_usage_for_identity(&identity), Some(40));
    assert_eq!(
        store.delete_cache_entries_for_handle_and_identity(
            &identity,
            "cache",
            id,
            &query("circle")
        )?,
        Some(true)
    );
    assert_eq!(
        store
            .cache_entries_for_handle_and_identity(&identity, "cache", id)
            .unwrap()[0]
            .response
            .body,
        b"square"
    );
    assert_eq!(store.cache_usage_for_identity(&identity), Some(30));
    Ok(())
}

#[test]
fn cache_batch_distinguishes_absent_and_empty_vary_request_headers() -> Result<()> {
    let mut store = StorageBucketRegistry::default();
    let identity = store.open_bucket("https://batch.test", "bucket")?;
    let id = store
        .open_cache_handle_for_identity(&identity, "cache")?
        .unwrap();
    let url = "https://batch.test/item";
    let mut absent = operation(url, "", Some("x-shape"), "absent", 10);
    absent.request.headers.clear();
    let mut empty = operation(url, "", Some("x-shape"), "empty", 20);
    empty.request.headers = vec![("X-Shape".to_owned(), String::new())];
    assert_eq!(
        store.put_cache_batch_for_handle_and_identity(
            &identity,
            "cache",
            id,
            vec![absent, empty],
            0
        )?,
        StorageBucketCachePutOutcome::Stored
    );
    for (headers, body) in [
        (Vec::new(), b"absent".as_slice()),
        (
            vec![("x-shape".to_owned(), String::new())],
            b"empty".as_slice(),
        ),
    ] {
        let query = StorageBucketCacheQuery {
            request_url: url.to_owned(),
            method: "GET".to_owned(),
            headers,
            ignore_search: false,
            ignore_method: false,
            ignore_vary: false,
        };
        let matches = store
            .match_cache_entries_for_handle_and_identity(&identity, "cache", id, &query)
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].response.body, body);
    }
    assert_eq!(store.cache_usage_for_identity(&identity), Some(30));
    Ok(())
}

#[test]
fn cache_batch_rejects_asymmetric_duplicates_without_touching_existing_entries() -> Result<()> {
    let mut store = StorageBucketRegistry::default();
    let identity = store.open_bucket("https://batch.test", "bucket")?;
    let id = store
        .open_cache_handle_for_identity(&identity, "cache")?
        .unwrap();
    store.put_cache_batch_for_handle_and_identity(
        &identity,
        "cache",
        id,
        vec![operation("https://batch.test/item", "old", None, "old", 7)],
        0,
    )?;
    let before = store.cache_entries_for_handle_and_identity(&identity, "cache", id);
    let asymmetric = vec![
        operation(
            "https://batch.test/item",
            "circle",
            Some("x-shape"),
            "a",
            10,
        ),
        operation("https://batch.test/item", "square", Some("x-size"), "b", 10),
    ];
    for reverse in [false, true] {
        let mut operations = asymmetric.clone();
        if reverse {
            operations.reverse();
        }
        assert_eq!(
            store.put_cache_batch_for_handle_and_identity(&identity, "cache", id, operations, 0)?,
            StorageBucketCachePutOutcome::Duplicate
        );
        assert_eq!(
            store.cache_entries_for_handle_and_identity(&identity, "cache", id),
            before
        );
        assert_eq!(store.cache_usage_for_identity(&identity), Some(7));
    }
    Ok(())
}

#[test]
fn cache_batch_checks_combined_quota_and_rolls_back_replacements() -> Result<()> {
    let mut store = StorageBucketRegistry::default();
    let identity = store.open_bucket_with_options(
        "https://batch.test",
        "bucket",
        None,
        None,
        Some(100),
        None,
    )?;
    let id = store
        .open_cache_handle_for_identity(&identity, "cache")?
        .unwrap();
    store.put_cache_batch_for_handle_and_identity(
        &identity,
        "cache",
        id,
        vec![operation("https://batch.test/item", "old", None, "old", 40)],
        50,
    )?;
    let before = store.cache_entries_for_handle_and_identity(&identity, "cache", id);
    assert_eq!(
        store.put_cache_batch_for_handle_and_identity(
            &identity,
            "cache",
            id,
            vec![
                operation("https://batch.test/item", "new", None, "new", 30),
                operation("https://batch.test/another", "new", None, "new", 30),
            ],
            50
        )?,
        StorageBucketCachePutOutcome::QuotaExceeded {
            quota: 100,
            requested: 110
        }
    );
    assert_eq!(
        store.cache_entries_for_handle_and_identity(&identity, "cache", id),
        before
    );
    assert_eq!(store.cache_usage_for_identity(&identity), Some(40));
    assert_eq!(
        store.put_cache_batch_for_handle_and_identity(
            &identity,
            "cache",
            id,
            vec![
                operation("https://batch.test/item", "new", None, "new", 20),
                operation("https://batch.test/another", "new", None, "new", 30),
            ],
            50
        )?,
        StorageBucketCachePutOutcome::Stored
    );
    assert_eq!(store.cache_usage_for_identity(&identity), Some(50));
    Ok(())
}

#[test]
fn cache_batch_reopens_same_url_variants_and_reads_legacy_url_maps() -> Result<()> {
    let temp = TempStorePath::new("cache-batch-persistence");
    let root = temp.cache_root();
    let expected = {
        let shared = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
        let mut store = shared.lock();
        let identity = store.open_bucket("https://batch.test", "bucket")?;
        let id = store
            .open_cache_handle_for_identity(&identity, "cache")?
            .unwrap();
        store.put_cache_batch_for_handle_and_identity(
            &identity,
            "cache",
            id,
            vec![
                operation(
                    "https://batch.test/item",
                    "circle",
                    Some("x-shape"),
                    "a\0b",
                    20,
                ),
                operation(
                    "https://batch.test/item",
                    "square",
                    Some("x-shape"),
                    "square",
                    30,
                ),
            ],
            0,
        )?;
        store
            .cache_entries_for_handle_and_identity(&identity, "cache", id)
            .unwrap()
    };
    let shared = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = shared.lock();
    let identity = store.open_bucket("https://batch.test", "bucket")?;
    assert_eq!(
        store.cache_entries_for_identity(&identity, "cache"),
        Some(expected)
    );
    assert_eq!(store.cache_usage_for_identity(&identity), Some(50));
    let path = super::super::storage_bucket_cache_file_path(
        &root,
        "https://batch.test",
        identity.bucket_id,
        "cache",
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    assert_eq!(json["version"], 2);
    assert_eq!(json["entries"].as_array().unwrap().len(), 2);
    let legacy = temp.path.with_extension("legacy.json");
    fs::write(&legacy, br#"{"version":1,"entries":{"https://batch.test/old":{"usageBytes":11,"status":200,"statusText":"OK","headers":[],"bodyBase64":"b2xk"}}}"#)?;
    let entries = load_storage_bucket_cache_file(&legacy)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].request_url, "https://batch.test/old");
    assert_eq!(entries[0].request.method, "GET");
    assert_eq!(entries[0].response.body, b"old");
    assert_eq!(entries[0].usage_bytes, 11);
    fs::remove_file(legacy)?;
    Ok(())
}

#[test]
fn cache_batch_persistent_write_failure_keeps_previous_memory_and_disk() -> Result<()> {
    let temp = TempStorePath::new("cache-batch-io-rollback");
    let root = temp.cache_root();
    let shared = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &root)?;
    let mut store = shared.lock();
    let identity = store.open_bucket("https://batch.test", "bucket")?;
    let id = store
        .open_cache_handle_for_identity(&identity, "cache")?
        .unwrap();
    store.put_cache_batch_for_handle_and_identity(
        &identity,
        "cache",
        id,
        vec![operation("https://batch.test/item", "old", None, "old", 7)],
        0,
    )?;
    let before = store.cache_entries_for_handle_and_identity(&identity, "cache", id);
    let path = super::super::storage_bucket_cache_file_path(
        &root,
        "https://batch.test",
        identity.bucket_id,
        "cache",
    );
    let bytes = fs::read(&path)?;
    let super::super::StorageBucketBackend::Json(backend) = &mut store.backend else {
        unreachable!()
    };
    // A regular file cannot be the parent of the metadata destination.
    let original_path = backend.path.clone();
    backend.path = temp.path.join("blocked.json");
    assert!(
        store
            .put_cache_batch_for_handle_and_identity(
                &identity,
                "cache",
                id,
                vec![
                    operation("https://batch.test/item", "new", None, "new", 10),
                    operation("https://batch.test/other", "new", None, "other", 10)
                ],
                0
            )
            .is_err()
    );
    assert_eq!(
        store.cache_entries_for_handle_and_identity(&identity, "cache", id),
        before
    );
    assert_eq!(fs::read(path)?, bytes);
    let super::super::StorageBucketBackend::Json(backend) = &mut store.backend else {
        unreachable!()
    };
    backend.path = original_path;
    Ok(())
}

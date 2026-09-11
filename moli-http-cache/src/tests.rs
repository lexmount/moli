use std::{fs, path::PathBuf};

use anyhow::Result;
use url::Url;

use super::*;
use crate::{
    metadata::{
        HttpCacheFormatVersion, META_FILE, read_metadata_file, read_published_body_file,
        touch_metadata_last_used_if_body_matches,
    },
    path_safety::safe_body_file_name,
    time::unique_suffix,
};

fn temp_root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("moli-http-cache-test-{name}-{}", unique_suffix()));
    let _ = fs::remove_dir_all(&root);
    root
}

fn test_metadata(
    url: &str,
    status: u16,
    headers: Vec<(String, String)>,
    stored_at_unix_ms: u64,
    expires_at_unix_ms: Option<u64>,
) -> HttpCacheEntryMetadata {
    HttpCacheEntryMetadata::new(
        url.to_owned(),
        url.to_owned(),
        status,
        headers,
        stored_at_unix_ms,
        expires_at_unix_ms,
        Vec::new(),
    )
}

fn load_test_entry(store: &HttpCacheStore, key: &str) -> Result<Option<HttpCachedEntry>> {
    let Some(entry) = store.load_reader(key)? else {
        return Ok(None);
    };
    entry.try_into_bytes(1024 * 1024)
}

#[test]
fn cache_freshness_max_age_accounts_for_date_and_age() {
    assert_eq!(
        cache_expires_at_unix_ms(100_000, Some(60), None, Some(70_000), 10),
        Some(130_000)
    );
    assert_eq!(
        cache_expires_at_unix_ms(100_000, Some(60), None, Some(99_000), 80),
        Some(100_000)
    );
}

#[test]
fn cache_freshness_uses_expires_when_max_age_is_absent() {
    assert_eq!(
        cache_expires_at_unix_ms(100_000, None, Some(150_000), Some(90_000), 20),
        Some(150_000)
    );
}

#[test]
fn request_cache_control_validation_honors_no_cache_and_max_age_zero() {
    assert!(request_cache_control_requires_validation("no-cache"));
    assert!(request_cache_control_requires_validation("max-age=0"));
    assert!(request_cache_control_requires_validation("max-age=\"0\""));
    assert!(!request_cache_control_requires_validation("max-age=60"));
}

#[test]
fn request_pragma_validation_only_honors_no_cache() {
    assert!(request_pragma_requires_validation("no-cache"));
    assert!(request_pragma_requires_validation("foo, no-cache"));
    assert!(!request_pragma_requires_validation("max-age=0"));
    assert!(!request_header_requires_validation("pragma", "max-age=0"));
    assert!(request_header_requires_validation(
        "cache-control",
        "max-age=0"
    ));
}

#[test]
fn response_vary_header_names_merges_all_header_fields() {
    let headers = vec![
        ("Vary".to_owned(), "Accept-Encoding, User-Agent".to_owned()),
        (
            "vary".to_owned(),
            "accept-encoding, Accept-Language".to_owned(),
        ),
    ];

    assert_eq!(
        response_vary_header_names(&headers),
        Some(vec![
            "accept-encoding".to_owned(),
            "user-agent".to_owned(),
            "accept-language".to_owned()
        ])
    );
}

#[test]
fn response_vary_header_names_rejects_wildcard() {
    let headers = vec![("Vary".to_owned(), "Accept-Encoding, *".to_owned())];

    assert_eq!(response_vary_header_names(&headers), None);
}

#[test]
fn cacheable_response_parts_policy_rejects_unsafe_response_parts() -> Result<()> {
    let request_url = Url::parse("https://example.test/app.js")?;
    let other_url = Url::parse("https://example.test/other.js")?;

    assert!(cacheable_response_parts_policy(&request_url, &request_url, 200, &[], false).is_some());
    assert!(cacheable_response_parts_policy(&request_url, &request_url, 199, &[], false).is_none());
    assert!(cacheable_response_parts_policy(&request_url, &request_url, 300, &[], false).is_none());
    assert!(cacheable_response_parts_policy(&request_url, &request_url, 301, &[], false).is_some());
    assert!(cacheable_response_parts_policy(&request_url, &request_url, 200, &[], true).is_none());
    assert!(cacheable_response_parts_policy(&request_url, &other_url, 200, &[], false).is_none());
    assert!(
        cacheable_response_parts_policy(
            &request_url,
            &request_url,
            200,
            &[("set-cookie".to_owned(), "sid=1".to_owned())],
            false
        )
        .is_none()
    );
    assert!(
        cacheable_response_parts_policy(
            &request_url,
            &request_url,
            200,
            &[("cache-control".to_owned(), "no-store".to_owned())],
            false
        )
        .is_none()
    );

    Ok(())
}

#[test]
fn validation_headers_use_cached_validators() {
    let headers = vec![
        ("etag".to_owned(), "\"v1\"".to_owned()),
        (
            "last-modified".to_owned(),
            "Wed, 21 Oct 2015 07:28:00 GMT".to_owned(),
        ),
    ];

    assert_eq!(
        validation_headers_from_headers(&headers),
        vec![
            ("If-None-Match".to_owned(), "\"v1\"".to_owned()),
            (
                "If-Modified-Since".to_owned(),
                "Wed, 21 Oct 2015 07:28:00 GMT".to_owned()
            )
        ]
    );
}

#[test]
fn not_modified_merge_skips_hop_by_hop_and_connection_nominated_headers() {
    let cached = vec![
        ("cache-control".to_owned(), "max-age=60".to_owned()),
        ("x-old".to_owned(), "old".to_owned()),
    ];
    let not_modified = vec![
        ("cache-control".to_owned(), "max-age=120".to_owned()),
        ("connection".to_owned(), "x-transient".to_owned()),
        ("x-transient".to_owned(), "drop".to_owned()),
        ("content-length".to_owned(), "0".to_owned()),
        ("etag".to_owned(), "\"v2\"".to_owned()),
    ];

    assert_eq!(
        merge_not_modified_headers(&cached, &not_modified),
        vec![
            ("cache-control".to_owned(), "max-age=120".to_owned()),
            ("x-old".to_owned(), "old".to_owned()),
            ("etag".to_owned(), "\"v2\"".to_owned())
        ]
    );
}

#[test]
fn streaming_writer_publishes_metadata_after_body() -> Result<()> {
    let root = temp_root("publish");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    let mut writer = store.create_body_writer(&key)?;
    writer.write_all(b"hello ")?;
    writer.write_all(b"cache")?;
    writer.finish(test_metadata(
        "http://example.test/cache",
        200,
        vec![("cache-control".to_owned(), "max-age=60".to_owned())],
        1,
        Some(2),
    ))?;

    let cached = load_test_entry(&store, &key)?.expect("entry should be readable");
    assert_eq!(cached.metadata.status, 200);
    assert_eq!(cached.body, b"hello cache");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn cached_entry_reader_rejects_oversized_body_before_materialization() -> Result<()> {
    let root = temp_root("reader-limit");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"hello cache",
    )?;

    let entry = store
        .load_reader(&key)?
        .expect("entry should be readable as a stream");
    assert!(entry.try_into_bytes(4)?.is_none());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn dropped_writer_leaves_no_readable_entry() -> Result<()> {
    let root = temp_root("drop");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);

    {
        let mut writer = store.create_body_writer(&key)?;
        writer.write_all(b"partial")?;
    }

    assert!(load_test_entry(&store, &key)?.is_none());
    assert!(
        !entry_dir.exists(),
        "dropping an unpublished writer should remove its empty entry dir"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn writer_rejects_body_larger_than_store_limit_without_publishing() -> Result<()> {
    let root = temp_root("entry-limit");
    let store = HttpCacheStore::with_max_bytes(&root, Some(4));
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);

    let result = store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"too large",
    );

    assert!(result.is_err());
    assert!(load_test_entry(&store, &key)?.is_none());
    assert!(
        !entry_dir.exists(),
        "oversized unpublished body should be removed when the writer is dropped"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn streaming_writer_rejects_body_once_store_limit_is_exceeded() -> Result<()> {
    let root = temp_root("streaming-entry-limit");
    let store = HttpCacheStore::with_max_bytes(&root, Some(6));
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    {
        let mut writer = store.create_body_writer(&key)?;
        writer.write_all(b"hello")?;
        assert!(writer.write_all(b"!!").is_err());
    }

    assert!(load_test_entry(&store, &key)?.is_none());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn replacing_entry_removes_old_completed_body_stream() -> Result<()> {
    let root = temp_root("replace");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"first",
    )?;
    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 3, Some(4)),
        b"second",
    )?;

    let cached = load_test_entry(&store, &key)?.expect("entry should be readable");
    assert_eq!(cached.body, b"second");
    let body_files = fs::read_dir(root.join(format!("{key}.entry")))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("body."))
        .count();
    assert_eq!(body_files, 1);

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn binary_body_round_trips_without_text_loss() -> Result<()> {
    let root = temp_root("binary");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let body = [0x00, 0xff, 0x80, b'a', b'\n'];

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        &body,
    )?;

    let cached = load_test_entry(&store, &key)?.expect("entry should be readable");
    assert_eq!(cached.body, body);

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn load_reader_streams_completed_body() -> Result<()> {
    use std::io::Read;

    let root = temp_root("reader");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"streamed cached body",
    )?;

    let mut cached = store.load_reader(&key)?.expect("entry should be readable");
    let mut body = Vec::new();
    cached.body.read_to_end(&mut body)?;

    assert_eq!(cached.metadata.status, 200);
    assert_eq!(body, b"streamed cached body");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn refresh_loaded_entry_metadata_preserves_cached_body() -> Result<()> {
    let root = temp_root("refresh-metadata");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata(
            "http://example.test/cache",
            200,
            vec![
                ("cache-control".to_owned(), "max-age=0".to_owned()),
                ("etag".to_owned(), "\"v1\"".to_owned()),
            ],
            1,
            Some(2),
        ),
        b"cached body",
    )?;

    let mut loaded = store.load_reader(&key)?.expect("entry should be readable");
    loaded.metadata.headers = vec![
        ("cache-control".to_owned(), "max-age=60".to_owned()),
        ("etag".to_owned(), "\"v1\"".to_owned()),
    ];
    loaded.metadata.stored_at_unix_ms = 3;
    loaded.metadata.last_used_at_unix_ms = 3;
    loaded.metadata.expires_at_unix_ms = Some(63);
    store.refresh_loaded_entry_metadata(&key, &loaded.metadata)?;

    let cached = load_test_entry(&store, &key)?.expect("entry should remain readable");
    assert_eq!(cached.body, b"cached body");
    assert_eq!(
        cached.metadata.headers,
        vec![
            ("cache-control".to_owned(), "max-age=60".to_owned()),
            ("etag".to_owned(), "\"v1\"".to_owned()),
        ]
    );
    assert_eq!(cached.metadata.expires_at_unix_ms, Some(63));

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn stats_and_entries_are_empty_for_missing_root() -> Result<()> {
    let root = temp_root("missing-stats");
    let store = HttpCacheStore::new(&root);

    assert_eq!(store.entries()?, Vec::new());
    assert_eq!(store.stats()?, HttpCacheStats::default());

    Ok(())
}

#[test]
fn stats_and_entries_report_readable_cache_entries() -> Result<()> {
    let root = temp_root("stats-readable");
    let store = HttpCacheStore::new(&root);
    let first_key = HttpCacheStore::key_for_url("http://example.test/first");
    let second_key = HttpCacheStore::key_for_url("http://example.test/second");

    store.store_body(
        &first_key,
        test_metadata("http://example.test/first", 200, Vec::new(), 1, Some(2)),
        b"first",
    )?;
    store.store_body(
        &second_key,
        test_metadata("http://example.test/second", 201, Vec::new(), 3, Some(4)),
        b"second-body",
    )?;

    let mut entries = store.entries()?;
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].key, first_key);
    assert_eq!(entries[0].metadata.request_url, "http://example.test/first");
    assert_eq!(entries[0].body_len, 5);
    assert!(entries[0].entry_size_bytes >= entries[0].body_len);
    assert_eq!(entries[1].key, second_key);
    assert_eq!(entries[1].metadata.status, 201);
    assert_eq!(entries[1].body_len, 11);

    let stats = store.stats()?;
    assert_eq!(stats.entry_count, 2);
    assert_eq!(stats.unreadable_entry_count, 0);
    assert_eq!(stats.readable_body_bytes, 16);
    assert!(stats.total_bytes >= stats.readable_body_bytes);

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn stats_counts_unreadable_entry_directories() -> Result<()> {
    let root = temp_root("stats-unreadable");
    let store = HttpCacheStore::new(&root);
    let readable_key = HttpCacheStore::key_for_url("http://example.test/readable");
    let orphan_key = HttpCacheStore::key_for_url("http://example.test/orphan");

    store.store_body(
        &readable_key,
        test_metadata("http://example.test/readable", 200, Vec::new(), 1, Some(2)),
        b"readable",
    )?;
    let orphan_dir = root.join(format!("{orphan_key}.entry"));
    fs::create_dir_all(&orphan_dir)?;
    fs::write(orphan_dir.join("body.orphan.bin"), b"orphaned")?;

    assert_eq!(store.entries()?.len(), 1);
    let stats = store.stats()?;
    assert_eq!(stats.entry_count, 1);
    assert_eq!(stats.unreadable_entry_count, 1);
    assert_eq!(stats.readable_body_bytes, 8);
    assert!(
        stats.total_bytes >= 16,
        "stats should include readable and orphaned entry bytes: {stats:?}"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn remove_entries_matching_removes_only_matching_readable_entries() -> Result<()> {
    let root = temp_root("remove-matching");
    let store = HttpCacheStore::new(&root);
    let keep_key = HttpCacheStore::key_for_url("http://example.test/keep");
    let remove_key = HttpCacheStore::key_for_url("http://example.test/remove");
    let orphan_key = HttpCacheStore::key_for_url("http://example.test/orphan");

    store.store_body(
        &keep_key,
        test_metadata("http://example.test/keep", 200, Vec::new(), 1, Some(2)),
        b"keep",
    )?;
    store.store_body(
        &remove_key,
        test_metadata("http://example.test/remove", 404, Vec::new(), 3, Some(4)),
        b"remove",
    )?;
    let orphan_dir = root.join(format!("{orphan_key}.entry"));
    fs::create_dir_all(&orphan_dir)?;
    fs::write(orphan_dir.join("body.orphan.bin"), b"orphaned")?;

    let removed = store.remove_entries_matching(|info| info.metadata.status == 404)?;

    assert_eq!(removed, 1);
    assert!(load_test_entry(&store, &keep_key)?.is_some());
    assert!(load_test_entry(&store, &remove_key)?.is_none());
    assert!(
        orphan_dir.exists(),
        "predicate cleanup should not remove unreadable entries blindly"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn remove_entries_for_origin_matches_request_and_final_urls() -> Result<()> {
    let root = temp_root("remove-origin");
    let store = HttpCacheStore::new(&root);
    let request_origin_key = HttpCacheStore::key_for_url("http://app.test/request");
    let final_origin_key = HttpCacheStore::key_for_url("http://cdn.test/final");
    let other_key = HttpCacheStore::key_for_url("http://other.test/cache");

    store.store_body(
        &request_origin_key,
        HttpCacheEntryMetadata::new(
            "http://app.test/request".to_owned(),
            "http://cdn.test/request".to_owned(),
            200,
            Vec::new(),
            1,
            Some(2),
            Vec::new(),
        ),
        b"request-origin",
    )?;
    store.store_body(
        &final_origin_key,
        HttpCacheEntryMetadata::new(
            "http://source.test/final".to_owned(),
            "http://app.test/final".to_owned(),
            200,
            Vec::new(),
            3,
            Some(4),
            Vec::new(),
        ),
        b"final-origin",
    )?;
    store.store_body(
        &other_key,
        test_metadata("http://other.test/cache", 200, Vec::new(), 5, Some(6)),
        b"other",
    )?;

    let removed = store.remove_entries_for_origin(&Url::parse("http://app.test/page")?)?;

    assert_eq!(removed, 2);
    assert!(load_test_entry(&store, &request_origin_key)?.is_none());
    assert!(load_test_entry(&store, &final_origin_key)?.is_none());
    assert!(load_test_entry(&store, &other_key)?.is_some());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn reader_materialization_respects_explicit_limit() -> Result<()> {
    let root = temp_root("reader-limit");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"too large",
    )?;

    let cached = store.load_reader(&key)?.expect("entry should be readable");
    assert!(
        cached.try_into_bytes(3)?.is_none(),
        "materialization should fail closed when the caller's body limit is exceeded"
    );
    let cached = load_test_entry(&store, &key)?.expect("entry should still be readable");
    assert_eq!(cached.body, b"too large");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn zero_length_body_is_a_complete_entry() -> Result<()> {
    let root = temp_root("empty");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 204, Vec::new(), 1, Some(2)),
        b"",
    )?;

    let cached = load_test_entry(&store, &key)?.expect("empty body entry should read");
    assert!(cached.body.is_empty());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn metadata_is_not_visible_until_streaming_writer_finishes() -> Result<()> {
    let root = temp_root("metadata-last");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    let mut writer = store.create_body_writer(&key)?;
    writer.write_all(b"partial")?;
    assert!(
        load_test_entry(&store, &key)?.is_none(),
        "unpublished body files without metadata must be ignored"
    );
    writer.finish(test_metadata(
        "http://example.test/cache",
        200,
        Vec::new(),
        1,
        Some(2),
    ))?;

    let cached = load_test_entry(&store, &key)?.expect("finished writer should publish metadata");
    assert_eq!(cached.body, b"partial");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn missing_body_file_is_treated_as_incomplete_entry() -> Result<()> {
    let root = temp_root("missing-body");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);
    fs::create_dir_all(&entry_dir)?;
    let metadata = HttpCacheEntryMetadata {
        version: HttpCacheFormatVersion::default(),
        request_url: "http://example.test/cache".to_owned(),
        final_url: "http://example.test/cache".to_owned(),
        status: 200,
        status_text: None,
        headers: Vec::new(),
        stored_at_unix_ms: 1,
        last_used_at_unix_ms: 1,
        expires_at_unix_ms: Some(2),
        vary_headers: Vec::new(),
        body_file: "body.missing.bin".to_owned(),
    };
    fs::write(entry_dir.join(META_FILE), serde_json::to_vec(&metadata)?)?;

    assert!(
        load_test_entry(&store, &key)?.is_none(),
        "metadata without a body stream is equivalent to an incomplete cache entry"
    );
    assert!(
        !entry_dir.join(META_FILE).exists(),
        "metadata pointing at a missing body should be cleaned"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn corrupt_metadata_is_ignored_and_removed() -> Result<()> {
    let root = temp_root("corrupt-meta");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);
    fs::create_dir_all(&entry_dir)?;
    let meta_path = entry_dir.join(META_FILE);
    fs::write(&meta_path, b"{not-json")?;

    assert!(load_test_entry(&store, &key)?.is_none());
    assert!(
        !meta_path.exists(),
        "corrupt metadata should be removed so future reads do not keep reparsing it"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn unsupported_metadata_version_is_ignored_and_removed() -> Result<()> {
    let root = temp_root("unsupported-version");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);
    for version in [3, 99] {
        fs::create_dir_all(&entry_dir)?;
        let meta_path = entry_dir.join(META_FILE);
        fs::write(
            &meta_path,
            serde_json::to_vec(&serde_json::json!({
                "version": version,
                "request_url": "http://example.test/cache",
                "final_url": "http://example.test/cache",
                "status": 200,
                "headers": [],
                "stored_at_unix_ms": 1,
                "last_used_at_unix_ms": 1,
                "expires_at_unix_ms": 2,
                "vary_headers": [],
                "body_file": "body.unsupported.bin"
            }))?,
        )?;

        assert!(load_test_entry(&store, &key)?.is_none());
        assert!(
            !meta_path.exists(),
            "unsupported cache metadata version should be removed"
        );
    }

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn legacy_metadata_without_version_is_ignored_and_removed() -> Result<()> {
    let root = temp_root("missing-version");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);
    fs::create_dir_all(&entry_dir)?;
    let meta_path = entry_dir.join(META_FILE);
    fs::write(
        &meta_path,
        serde_json::to_vec(&serde_json::json!({
            "request_url": "http://example.test/cache",
            "final_url": "http://example.test/cache",
            "status": 200,
            "headers": [],
            "stored_at_unix_ms": 1,
            "last_used_at_unix_ms": 1,
            "expires_at_unix_ms": 2,
            "vary_headers": [],
            "body_file": "body.legacy.bin"
        }))?,
    )?;

    assert!(load_test_entry(&store, &key)?.is_none());
    assert!(
        !meta_path.exists(),
        "legacy cache metadata without a typed version should be removed"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn metadata_body_file_must_not_escape_entry_directory() -> Result<()> {
    let root = temp_root("path-traversal");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    let entry_dir = store.entry_dir(&key);
    fs::create_dir_all(&entry_dir)?;
    let meta_path = entry_dir.join(META_FILE);
    let metadata = HttpCacheEntryMetadata {
        version: HttpCacheFormatVersion::default(),
        request_url: "http://example.test/cache".to_owned(),
        final_url: "http://example.test/cache".to_owned(),
        status: 200,
        status_text: None,
        headers: Vec::new(),
        stored_at_unix_ms: 1,
        last_used_at_unix_ms: 1,
        expires_at_unix_ms: Some(2),
        vary_headers: Vec::new(),
        body_file: "../outside.bin".to_owned(),
    };
    fs::write(&meta_path, serde_json::to_vec(&metadata)?)?;

    assert!(load_test_entry(&store, &key)?.is_none());
    assert!(
        !meta_path.exists(),
        "unsafe metadata should be removed instead of being retried"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn body_file_metadata_must_be_one_safe_filename_component() {
    assert!(safe_body_file_name("body.123.bin"));
    assert!(!safe_body_file_name("../outside.bin"));
    assert!(!safe_body_file_name("body.parent/../outside.bin"));
    assert!(!safe_body_file_name("body.parent/inside.bin"));
    assert!(!safe_body_file_name("body.parent\\inside.bin"));
    assert!(!safe_body_file_name("/body.absolute.bin"));
    assert!(!safe_body_file_name("body.with-child.bin/extra"));
}

#[test]
fn public_cache_key_must_not_escape_root() -> Result<()> {
    let root = temp_root("unsafe-key");
    let store = HttpCacheStore::new(&root);
    let unsafe_keys = ["../outside", "abc/def", "abc\\def", "/absolute", "ABCDEF"];

    for key in unsafe_keys {
        assert!(!store.contains_entry_path(key));
        assert!(load_test_entry(&store, key)?.is_none());
        assert!(
            store.create_body_writer(key).is_err(),
            "unsafe public key `{key}` must not create a path"
        );
    }

    assert!(!root.join("outside.entry").exists());
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn contains_entry_path_requires_directory_entry() -> Result<()> {
    let root = temp_root("contains-dir");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");
    fs::create_dir_all(&root)?;
    fs::write(store.entry_dir(&key), b"not a directory")?;

    assert!(
        !store.contains_entry_path(&key),
        "regular files must not be reported as cache entry directories"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn clear_removes_cache_entries_and_preserves_unrelated_root_files() -> Result<()> {
    let root = temp_root("clear");
    let store = HttpCacheStore::new(&root);
    let first_key = HttpCacheStore::key_for_url("http://example.test/first");
    let second_key = HttpCacheStore::key_for_url("http://example.test/second");
    let unrelated_file = root.join("owner.lock");
    let unrelated_dir = root.join("not-cache");

    store.store_body(
        &first_key,
        test_metadata("http://example.test/first", 200, Vec::new(), 1, Some(2)),
        b"first",
    )?;
    store.store_body(
        &second_key,
        test_metadata("http://example.test/second", 200, Vec::new(), 3, Some(4)),
        b"second",
    )?;
    fs::write(&unrelated_file, b"keep")?;
    fs::create_dir_all(&unrelated_dir)?;

    store.clear()?;

    assert!(load_test_entry(&store, &first_key)?.is_none());
    assert!(load_test_entry(&store, &second_key)?.is_none());
    assert!(unrelated_file.exists());
    assert!(unrelated_dir.exists());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn clear_is_noop_when_root_is_missing() -> Result<()> {
    let root = temp_root("clear-missing");
    let store = HttpCacheStore::new(&root);

    store.clear()?;

    assert!(!root.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn clear_does_not_follow_entry_symlinks() -> Result<()> {
    use std::os::unix::fs::symlink;

    let root = temp_root("clear-symlink");
    let outside = temp_root("clear-symlink-outside");
    fs::create_dir_all(&root)?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("owned.txt"), b"outside")?;
    symlink(&outside, root.join("linked.entry"))?;

    let store = HttpCacheStore::new(&root);
    store.clear()?;

    assert!(
        outside.join("owned.txt").exists(),
        "clear must not recurse through a cache-shaped symlink"
    );
    assert!(root.join("linked.entry").exists());

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
    Ok(())
}

#[test]
fn generated_cache_key_is_safe_filename_stem() {
    let key = HttpCacheStore::key_for_url("https://example.test/path?query#fragment");

    assert_eq!(key.len(), 16);
    assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(key, key.to_ascii_lowercase());
}

#[test]
fn replacing_entry_does_not_remove_concurrent_unpublished_body() -> Result<()> {
    let root = temp_root("concurrent-replace");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"first",
    )?;

    let mut pending_writer = store.create_body_writer(&key)?;
    pending_writer.write_all(b"pending")?;

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 3, Some(4)),
        b"second",
    )?;

    pending_writer.finish(test_metadata(
        "http://example.test/cache",
        200,
        Vec::new(),
        5,
        Some(6),
    ))?;

    let cached = load_test_entry(&store, &key)?.expect("latest writer should publish");
    assert_eq!(cached.body, b"pending");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn store_prunes_oldest_entries_when_quota_is_exceeded() -> Result<()> {
    let root = temp_root("quota");
    let store = HttpCacheStore::with_max_bytes(&root, Some(3_000));
    let first_key = HttpCacheStore::key_for_url("http://example.test/first");
    let second_key = HttpCacheStore::key_for_url("http://example.test/second");
    let third_key = HttpCacheStore::key_for_url("http://example.test/third");
    let body = vec![b'x'; 1024];

    store.store_body(
        &first_key,
        test_metadata("http://example.test/first", 200, Vec::new(), 1, Some(2)),
        &body,
    )?;
    store.store_body(
        &second_key,
        test_metadata("http://example.test/second", 200, Vec::new(), 2, Some(3)),
        &body,
    )?;
    store.store_body(
        &third_key,
        test_metadata("http://example.test/third", 200, Vec::new(), 3, Some(4)),
        &body,
    )?;

    assert!(
        load_test_entry(&store, &first_key)?.is_none(),
        "oldest entry should be evicted first"
    );
    assert!(load_test_entry(&store, &second_key)?.is_some());
    assert!(load_test_entry(&store, &third_key)?.is_some());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn trim_to_max_bytes_prunes_existing_cache_root() -> Result<()> {
    let root = temp_root("startup-trim");
    let seed_store = HttpCacheStore::new(&root);
    let first_key = HttpCacheStore::key_for_url("http://example.test/first");
    let second_key = HttpCacheStore::key_for_url("http://example.test/second");
    let third_key = HttpCacheStore::key_for_url("http://example.test/third");
    let body = vec![b'x'; 1024];

    seed_store.store_body(
        &first_key,
        test_metadata("http://example.test/first", 200, Vec::new(), 1, Some(2)),
        &body,
    )?;
    seed_store.store_body(
        &second_key,
        test_metadata("http://example.test/second", 200, Vec::new(), 2, Some(3)),
        &body,
    )?;
    seed_store.store_body(
        &third_key,
        test_metadata("http://example.test/third", 200, Vec::new(), 3, Some(4)),
        &body,
    )?;

    let trimming_store = HttpCacheStore::with_max_bytes(&root, Some(3_000));
    trimming_store.trim_to_max_bytes();

    assert!(load_test_entry(&trimming_store, &first_key)?.is_none());
    assert!(load_test_entry(&trimming_store, &second_key)?.is_some());
    assert!(load_test_entry(&trimming_store, &third_key)?.is_some());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn cache_hit_touches_last_used_for_eviction_order() -> Result<()> {
    let root = temp_root("quota-touch");
    let store = HttpCacheStore::with_max_bytes(&root, Some(3_000));
    let first_key = HttpCacheStore::key_for_url("http://example.test/first");
    let second_key = HttpCacheStore::key_for_url("http://example.test/second");
    let third_key = HttpCacheStore::key_for_url("http://example.test/third");
    let body = vec![b'x'; 1024];

    store.store_body(
        &first_key,
        test_metadata("http://example.test/first", 200, Vec::new(), 1, Some(2)),
        &body,
    )?;
    store.store_body(
        &second_key,
        test_metadata("http://example.test/second", 200, Vec::new(), 2, Some(3)),
        &body,
    )?;
    let first = load_test_entry(&store, &first_key)?.expect("first entry should load");
    store.touch_loaded_entry(&first_key, &first.metadata)?;
    store.store_body(
        &third_key,
        test_metadata("http://example.test/third", 200, Vec::new(), 3, Some(4)),
        &body,
    )?;

    assert!(load_test_entry(&store, &first_key)?.is_some());
    assert!(
        load_test_entry(&store, &second_key)?.is_none(),
        "entry not touched since storage should be evicted before the cache hit"
    );
    assert!(load_test_entry(&store, &third_key)?.is_some());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn cache_hit_touch_does_not_restore_replaced_body_metadata() -> Result<()> {
    let root = temp_root("touch-race");
    let store = HttpCacheStore::new(&root);
    let key = HttpCacheStore::key_for_url("http://example.test/cache");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 1, Some(2)),
        b"first",
    )?;
    let entry_dir = store.entry_dir(&key);
    let old_body_file = read_published_body_file(&entry_dir).expect("old body file");
    let metadata_read_by_cache_hit =
        read_metadata_file(&entry_dir.join(META_FILE)).expect("metadata should parse");

    store.store_body(
        &key,
        test_metadata("http://example.test/cache", 200, Vec::new(), 3, Some(4)),
        b"second",
    )?;
    let new_body_file = read_published_body_file(&entry_dir).expect("new body file");
    assert_ne!(old_body_file, new_body_file);

    touch_metadata_last_used_if_body_matches(&entry_dir, &metadata_read_by_cache_hit.body_file, 9)?;

    let metadata = read_metadata_file(&entry_dir.join(META_FILE)).expect("metadata should parse");
    assert_eq!(metadata.body_file, new_body_file);
    let cached = load_test_entry(&store, &key)?.expect("entry should remain readable");
    assert_eq!(cached.body, b"second");

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test]
fn quota_prunes_orphaned_entry_directories() -> Result<()> {
    let root = temp_root("quota-orphan");
    let store = HttpCacheStore::with_max_bytes(&root, Some(2_500));
    let orphan_key = HttpCacheStore::key_for_url("http://example.test/orphan");
    let live_key = HttpCacheStore::key_for_url("http://example.test/live");
    let orphan_dir = root.join(format!("{orphan_key}.entry"));
    fs::create_dir_all(&orphan_dir)?;
    fs::write(orphan_dir.join("body.orphan.bin"), vec![b'o'; 2_048])?;

    store.store_body(
        &live_key,
        test_metadata("http://example.test/live", 200, Vec::new(), 1, Some(2)),
        &[b'l'; 1_024],
    )?;

    assert!(
        !orphan_dir.exists(),
        "orphaned or unreadable entry directories must still count against quota"
    );
    assert!(load_test_entry(&store, &live_key)?.is_some());

    let _ = fs::remove_dir_all(root);
    Ok(())
}

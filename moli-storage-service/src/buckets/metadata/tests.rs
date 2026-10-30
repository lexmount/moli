use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "moli-bucket-publication-{name}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )))
    }

    fn path(&self) -> PathBuf {
        self.0.join("buckets.json")
    }

    fn open(&self) -> SharedStorageBucketStore {
        new_shared_json_storage_bucket_store_with_cache_root(self.path(), self.0.join("cache"))
            .unwrap()
    }

    fn fail_publication(&self) {
        fs::rename(self.path(), self.0.join("backup.json")).unwrap();
        fs::create_dir(self.path()).unwrap();
    }

    fn restore_publication(&self) {
        fs::remove_dir(self.path()).unwrap();
        fs::rename(self.0.join("backup.json"), self.path()).unwrap();
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failed_bucket_creation_never_exposes_an_identity_or_advances_the_allocator() {
    let root = Root::new("creation");
    let store = root.open();
    fs::create_dir_all(root.path()).unwrap();
    assert!(store.lock().open_bucket("owner", "new").is_err());
    assert!(store.lock().bucket_id("owner", "new").is_none());
    assert!(store.lock().keys("owner").is_empty());
    assert_eq!(store.lock().memory().next_bucket_id, 0);
    fs::remove_dir(root.path()).unwrap();
    let first = store.lock().open_bucket("owner", "new").unwrap();
    assert_eq!(first.bucket_id().get(), 1);
    drop(store);
    let reopened = root.open();
    assert_eq!(reopened.lock().bucket_identity("owner", "new"), Some(first));
    assert_eq!(
        reopened
            .lock()
            .open_bucket("owner", "next")
            .unwrap()
            .bucket_id()
            .get(),
        2
    );
}

#[test]
fn failed_tombstone_and_expiration_publication_preserve_the_live_identity() {
    let root = Root::new("delete");
    let store = root.open();
    let identity = store.lock().open_bucket("owner", "kept").unwrap();
    root.fail_publication();
    assert!(store.lock().delete_bucket("owner", "kept").is_err());
    assert!(
        store
            .lock()
            .set_bucket_expires_for_identity(&identity, Some(1.0))
            .is_err()
    );
    assert!(store.lock().bucket_identity_is_live(&identity));
    assert_eq!(
        store.lock().bucket_expires_for_identity(&identity),
        Some(None)
    );
    assert!(store.lock().pending_deletions().is_empty());
    root.restore_publication();
    assert_eq!(
        store.lock().delete_bucket("owner", "kept").unwrap(),
        Some(identity.clone())
    );
    assert!(!store.lock().bucket_identity_is_live(&identity));
    root.fail_publication();
    assert!(store.lock().finish_bucket_deletion(&identity).is_err());
    assert_eq!(
        store.lock().pending_deletions(),
        std::slice::from_ref(&identity)
    );
    root.restore_publication();
    drop(store);
    let reopened = root.open();
    assert_eq!(
        reopened.lock().pending_deletions(),
        std::slice::from_ref(&identity)
    );
    assert!(reopened.lock().finish_bucket_deletion(&identity).unwrap());
    let replacement = reopened.lock().open_bucket("owner", "kept").unwrap();
    assert!(replacement.bucket_id() > identity.bucket_id());
}

#[test]
fn metadata_candidates_preserve_cache_payloads_and_handle_identity_without_cloning() {
    let root = Root::new("cache");
    let store = root.open();
    let identity = store.lock().open_bucket("owner", "kept").unwrap();
    let cache_id = store
        .lock()
        .open_cache_handle_for_identity(&identity, "assets")
        .unwrap()
        .unwrap();
    let mut registry = store.lock();
    let metadata = registry.metadata_for_identity_mut(&identity).unwrap();
    let cache_name = StorageBucketCacheName::from("assets");
    metadata
        .cache_storage
        .get_mut(&cache_name)
        .unwrap()
        .push(StorageBucketCacheEntry {
            request_url: "https://a.test/resource".into(),
            usage_bytes: 1_048_576,
            request: StorageBucketCachedRequest::default(),
            response: StorageBucketCachedResponse {
                cors_exposed_header_names: None,
                response_type: "default".into(),
                url: "https://a.test/resource".into(),
                redirected: false,
                status: 200,
                status_text: "OK".into(),
                headers: vec![],
                body: vec![17; 1_048_576],
            },
            insertion_order: 0,
        });
    let pointer = metadata.cache_storage[&cache_name][0]
        .response
        .body
        .as_ptr();
    registry
        .set_bucket_expires_for_identity(&identity, Some(123.0))
        .unwrap();
    let metadata = registry.metadata_for_identity(&identity).unwrap();
    assert_eq!(metadata.cache_instance_ids[&cache_name], cache_id);
    assert_eq!(
        metadata.cache_storage[&cache_name][0]
            .response
            .body
            .as_ptr(),
        pointer
    );
    assert_eq!(
        registry.cache_usage_for_identity(&identity),
        Some(1_048_576)
    );
    let snapshot = registry.memory().clone_metadata();
    assert!(snapshot.origins["owner"]["kept"].cache_storage.is_empty());
    assert!(
        snapshot.origins["owner"]["kept"]
            .cache_instance_ref_counts
            .is_empty()
    );
}

#[test]
fn uncertain_metadata_publication_fences_writes_until_reopen_reads_the_published_allocator() {
    let root = Root::new("uncertain");
    let store = root.open();
    let old = store.lock().open_bucket("owner", "old").unwrap();
    let mut registry = store.lock();
    let mut candidate = StorageBucketRegistry {
        backend: StorageBucketBackend::Memory(registry.memory().clone_metadata()),
        storage_service: registry.storage_service(),
        indexed_db_manager: None,
    };
    let published = candidate.open_bucket("owner", "published").unwrap();
    let StorageBucketBackend::Json(json) = &mut registry.backend else {
        unreachable!()
    };
    // Model a lost directory-sync acknowledgement: disk may contain this
    // candidate while the registry still contains the previous publication.
    json.publish_metadata(candidate.memory()).unwrap();
    json.metadata_publication_uncertain = true;
    assert!(!registry.bucket_locator_is_live(&old.locator()));
    assert!(
        registry
            .bucket_quota_and_cache_usage(&old.locator())
            .is_none()
    );
    assert!(registry.open_bucket("owner", "retry").is_err());
    assert!(registry.delete_bucket("owner", "old").is_err());
    assert!(
        registry
            .set_bucket_expires_for_identity(&old, Some(1.0))
            .is_err()
    );
    drop(registry);
    drop(store);
    let reopened = root.open();
    assert_eq!(
        reopened.lock().bucket_identity("owner", "published"),
        Some(published.clone())
    );
    assert!(
        reopened
            .lock()
            .open_bucket("owner", "retry")
            .unwrap()
            .bucket_id()
            > published.bucket_id()
    );
}

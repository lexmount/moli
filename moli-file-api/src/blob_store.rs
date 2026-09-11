use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;
use uuid::Builder as UuidBuilder;

/// Runtime id for one Blob backing-store entry.
pub type BlobId = u64;

/// A parsed Blob URL retains immutable data and the URL creator's access key.
/// Consumers check the key against the fetching environment separately.
#[derive(Clone, Debug)]
pub struct ObjectUrlEntry<AccessKey> {
    pub bytes: Arc<[u8]>,
    pub mime_type: String,
    pub access_key: Option<AccessKey>,
}

#[derive(Clone, Debug)]
struct BlobState<OwnerId, PartitionId> {
    owner_id: Option<OwnerId>,
    partition_id: Option<PartitionId>,
    uuid: String,
    bytes: Arc<[u8]>,
    mime_type: String,
    wrapper_refs: usize,
    reader_refs: usize,
    object_url_refs: usize,
}

#[derive(Debug)]
struct BlobEntries<OwnerId, PartitionId> {
    by_id: HashMap<BlobId, BlobState<OwnerId, PartitionId>>,
    ids_by_uuid: HashMap<String, BlobId>,
}

impl<OwnerId, PartitionId> Default for BlobEntries<OwnerId, PartitionId> {
    fn default() -> Self {
        Self {
            by_id: HashMap::new(),
            ids_by_uuid: HashMap::new(),
        }
    }
}

#[derive(Debug)]
struct ObjectUrlState<OwnerId, AccessKey> {
    owner_id: Option<OwnerId>,
    lifetime_id: Option<u64>,
    blob_id: BlobId,
    access_key: Option<AccessKey>,
}

/// Renderer-neutral Blob and object URL backing store.
///
/// The store tracks bytes, MIME type, object URL mappings, and simple reference
/// counts. The embedding layer owns JS wrappers and calls the retain/release
/// hooks from its finalizers.
#[derive(Debug)]
pub struct BlobStore<OwnerId, PartitionId, AccessKey = ()> {
    blobs: Mutex<BlobEntries<OwnerId, PartitionId>>,
    next_blob_id: AtomicU64,
    object_urls: Mutex<HashMap<String, ObjectUrlState<OwnerId, AccessKey>>>,
}

impl<OwnerId, PartitionId, AccessKey> Default for BlobStore<OwnerId, PartitionId, AccessKey> {
    fn default() -> Self {
        Self {
            blobs: Mutex::default(),
            next_blob_id: AtomicU64::new(1),
            object_urls: Mutex::default(),
        }
    }
}

impl<OwnerId, PartitionId, AccessKey> BlobStore<OwnerId, PartitionId, AccessKey>
where
    OwnerId: Copy + Eq + Hash,
    PartitionId: Eq,
{
    /// Create a Blob backing-store entry with one wrapper reference.
    pub fn create_blob(
        &self,
        owner_id: Option<OwnerId>,
        partition_id: Option<PartitionId>,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> BlobId {
        let blob_id = self.next_blob_id.fetch_add(1, Ordering::Relaxed).max(1);
        let mut blobs = self.blobs.lock();
        let uuid = loop {
            let candidate = random_uuid();
            if !blobs.ids_by_uuid.contains_key(&candidate) {
                break candidate;
            }
        };
        blobs.ids_by_uuid.insert(uuid.clone(), blob_id);
        blobs.by_id.insert(
            blob_id,
            BlobState {
                owner_id,
                partition_id,
                uuid,
                bytes: bytes.into(),
                mime_type,
                wrapper_refs: 1,
                reader_refs: 0,
                object_url_refs: 0,
            },
        );
        blob_id
    }

    /// Return a copy of the Blob bytes.
    pub fn blob_bytes(&self, blob_id: BlobId) -> Option<Vec<u8>> {
        self.blobs
            .lock()
            .by_id
            .get(&blob_id)
            .map(|blob| blob.bytes.to_vec())
    }

    /// Return the stable DevTools UUID for a Blob.
    pub fn blob_uuid(&self, blob_id: BlobId) -> Option<String> {
        self.blobs
            .lock()
            .by_id
            .get(&blob_id)
            .map(|blob| blob.uuid.clone())
    }

    /// Return a copy of Blob bytes addressed by its DevTools UUID in a partition.
    pub fn blob_bytes_by_uuid_in_partition(
        &self,
        uuid: &str,
        partition_id: &PartitionId,
    ) -> Option<Vec<u8>> {
        self.blob_shared_bytes_by_uuid_in_partition(uuid, partition_id)
            .map(|bytes| bytes.to_vec())
    }

    /// Return the shared Blob backing addressed by its DevTools UUID in a partition.
    pub fn blob_shared_bytes_by_uuid_in_partition(
        &self,
        uuid: &str,
        partition_id: &PartitionId,
    ) -> Option<Arc<[u8]>> {
        let blobs = self.blobs.lock();
        let blob_id = blobs.ids_by_uuid.get(uuid)?;
        blobs
            .by_id
            .get(blob_id)
            .filter(|blob| blob.partition_id.as_ref() == Some(partition_id))
            .map(|blob| blob.bytes.clone())
    }

    /// Return the Blob MIME type.
    pub fn blob_mime_type(&self, blob_id: BlobId) -> Option<String> {
        self.blobs
            .lock()
            .by_id
            .get(&blob_id)
            .map(|blob| blob.mime_type.clone())
    }

    /// Create an object URL for a Blob.
    pub fn create_object_url(
        &self,
        owner_id: Option<OwnerId>,
        blob_id: BlobId,
        origin: &str,
    ) -> Option<String> {
        self.create_object_url_with_access_key(owner_id, blob_id, origin, None)
    }

    /// Create an object URL tied to a more specific execution-context lifetime.
    pub fn create_object_url_with_lifetime(
        &self,
        owner_id: Option<OwnerId>,
        lifetime_id: Option<u64>,
        blob_id: BlobId,
        origin: &str,
    ) -> Option<String> {
        self.create_object_url_with_lifetime_and_access_key(owner_id, lifetime_id, blob_id, origin, None)
    }

    /// Create an object URL with its creator environment's access key.
    pub fn create_object_url_with_access_key(
        &self,
        owner_id: Option<OwnerId>,
        blob_id: BlobId,
        origin: &str,
        access_key: Option<AccessKey>,
    ) -> Option<String> {
        self.create_object_url_with_lifetime_and_access_key(owner_id, None, blob_id, origin, access_key)
    }

    /// Associate the URL's independent creator key and execution-context lifetime.
    pub fn create_object_url_with_lifetime_and_access_key(
        &self,
        owner_id: Option<OwnerId>,
        lifetime_id: Option<u64>,
        blob_id: BlobId,
        origin: &str,
        access_key: Option<AccessKey>,
    ) -> Option<String> {
        self.retain_blob_object_url_ref(blob_id)?;
        let mut object_urls = self.object_urls.lock();
        let object_url = loop {
            let candidate = format!("blob:{origin}/{}", random_uuid());
            if !object_urls.contains_key(&candidate) {
                break candidate;
            }
        };
        object_urls.insert(
            object_url.clone(),
            ObjectUrlState {
                owner_id,
                lifetime_id,
                blob_id,
                access_key,
            },
        );
        Some(object_url)
    }

    /// Revoke an object URL and release its Blob object-URL reference.
    pub fn revoke_object_url(&self, url: &str) -> bool {
        self.revoke_object_url_if(url, |_| true)
    }

    /// Revoke an object URL only when its creator's key matches the caller's.
    /// Missing keys are unauthorized; checking and removal are atomic.
    pub fn revoke_object_url_with_access_key(&self, url: &str, access_key: &AccessKey) -> bool
    where
        AccessKey: Eq,
    {
        self.revoke_object_url_if(url, |state| state.access_key.as_ref() == Some(access_key))
    }

    fn revoke_object_url_if(
        &self,
        url: &str,
        is_authorized: impl FnOnce(&ObjectUrlState<OwnerId, AccessKey>) -> bool,
    ) -> bool {
        let state = {
            let mut object_urls = self.object_urls.lock();
            if !object_urls.get(url).is_some_and(is_authorized) {
                return false;
            }
            object_urls.remove(url).expect("authorized entry is locked")
        };
        self.release_blob_object_url_ref(state.blob_id);
        true
    }

    /// Return object URL bytes and MIME type, excluding its fragment.
    pub fn object_url_bytes_and_type(&self, url: &str) -> Option<(Vec<u8>, String)> {
        let (bytes, mime_type) = self.object_url_shared_bytes_and_type(url)?;
        Some((bytes.to_vec(), mime_type))
    }

    /// Capture an object URL entry without copying its immutable body. The
    /// captured entry remains usable after revocation or creator teardown.
    pub fn object_url_shared_bytes_and_type(&self, url: &str) -> Option<(Arc<[u8]>, String)> {
        let url = url.split_once('#').map_or(url, |(url, _)| url);
        let object_urls = self.object_urls.lock();
        self.object_url_blob_data(object_urls.get(url)?.blob_id)
    }

    /// Atomically capture the Blob backing and the URL creator's access key.
    /// Fragments are excluded from lookup; the snapshot survives revocation.
    pub fn object_url_entry(&self, url: &str) -> Option<ObjectUrlEntry<AccessKey>>
    where
        AccessKey: Clone,
    {
        let url = url.split_once('#').map_or(url, |(url, _)| url);
        let object_urls = self.object_urls.lock();
        let state = object_urls.get(url)?;
        let (bytes, mime_type) = self.object_url_blob_data(state.blob_id)?;
        Some(ObjectUrlEntry {
            bytes,
            mime_type,
            access_key: state.access_key.clone(),
        })
    }

    fn object_url_blob_data(&self, blob_id: BlobId) -> Option<(Arc<[u8]>, String)> {
        let blobs = self.blobs.lock();
        let blob = blobs.by_id.get(&blob_id)?;
        Some((blob.bytes.clone(), blob.mime_type.clone()))
    }

    /// Return object URL body decoded lossily as text plus MIME type.
    pub fn object_url_body_and_type(&self, url: &str) -> Option<(String, String)> {
        let (bytes, mime_type) = self.object_url_bytes_and_type(url)?;
        Some((String::from_utf8_lossy(&bytes).into_owned(), mime_type))
    }

    /// Revoke a departing context's URLs while preserving bytes retained by
    /// wrappers, readers, or URLs created by another context.
    pub fn retire_owner_resources(&self, owner_id: OwnerId) {
        let retired = self.take_object_urls_if(|state| state.owner_id == Some(owner_id));
        for state in retired {
            self.release_blob_object_url_ref(state.blob_id);
        }
        for blob in self.blobs.lock().by_id.values_mut() {
            if blob.owner_id == Some(owner_id) {
                blob.owner_id = None;
            }
        }
    }

    /// Revoke one owner's URLs for an execution-context lifetime.
    /// Lifetime identifiers are local to each resource owner.
    pub fn cleanup_object_url_lifetime(&self, owner_id: OwnerId, lifetime_id: u64) -> usize {
        let removed_blob_ids = {
            let mut object_urls = self.object_urls.lock();
            let mut removed_blob_ids = Vec::new();
            object_urls.retain(|_, state| {
                if state.owner_id == Some(owner_id) && state.lifetime_id == Some(lifetime_id) {
                    removed_blob_ids.push(state.blob_id);
                    false
                } else {
                    true
                }
            });
            removed_blob_ids
        };
        let removed_count = removed_blob_ids.len();
        for blob_id in removed_blob_ids {
            self.release_blob_object_url_ref(blob_id);
        }
        removed_count
    }

    /// Remove Blob/object URL entries owned by a context.
    pub fn cleanup_owner_resources(&self, owner_id: OwnerId) {
        let removed_blob_ids = {
            let mut blobs = self.blobs.lock();
            let ids = blobs
                .by_id
                .iter()
                .filter_map(|(blob_id, blob)| (blob.owner_id == Some(owner_id)).then_some(*blob_id))
                .collect::<HashSet<_>>();
            for blob_id in &ids {
                if let Some(blob) = blobs.by_id.remove(blob_id) {
                    blobs.ids_by_uuid.remove(&blob.uuid);
                }
            }
            ids
        };

        let removed = self.take_object_urls_if(|state| {
            state.owner_id == Some(owner_id) || removed_blob_ids.contains(&state.blob_id)
        });
        for state in removed {
            if !removed_blob_ids.contains(&state.blob_id) {
                self.release_blob_object_url_ref(state.blob_id);
            }
        }
    }

    fn take_object_urls_if(
        &self,
        mut remove: impl FnMut(&ObjectUrlState<OwnerId, AccessKey>) -> bool,
    ) -> Vec<ObjectUrlState<OwnerId, AccessKey>> {
        // Removed URL state is returned so associated objects are released after the lock.
        self.object_urls
            .lock()
            .extract_if(|_, state| remove(state))
            .map(|(_, state)| state)
            .collect()
    }

    /// Retain a reader reference for a Blob.
    pub fn retain_blob_reader_ref(&self, blob_id: BlobId) {
        if let Some(blob) = self.blobs.lock().by_id.get_mut(&blob_id) {
            blob.reader_refs = blob.reader_refs.saturating_add(1);
        }
    }

    /// Release a wrapper reference and remove the Blob if no references remain.
    pub fn release_blob_wrapper_ref(&self, blob_id: BlobId) {
        self.release_blob_ref(blob_id, |blob| {
            blob.wrapper_refs = blob.wrapper_refs.saturating_sub(1);
        });
    }

    /// Release a reader reference and remove the Blob if no references remain.
    pub fn release_blob_reader_ref(&self, blob_id: BlobId) {
        self.release_blob_ref(blob_id, |blob| {
            blob.reader_refs = blob.reader_refs.saturating_sub(1);
        });
    }

    fn retain_blob_object_url_ref(&self, blob_id: BlobId) -> Option<()> {
        let mut blobs = self.blobs.lock();
        let blob = blobs.by_id.get_mut(&blob_id)?;
        blob.object_url_refs = blob.object_url_refs.saturating_add(1);
        Some(())
    }

    fn release_blob_object_url_ref(&self, blob_id: BlobId) {
        self.release_blob_ref(blob_id, |blob| {
            blob.object_url_refs = blob.object_url_refs.saturating_sub(1);
        });
    }

    fn release_blob_ref(
        &self,
        blob_id: BlobId,
        release: impl FnOnce(&mut BlobState<OwnerId, PartitionId>),
    ) {
        let mut blobs = self.blobs.lock();
        let Some(blob) = blobs.by_id.get_mut(&blob_id) else {
            return;
        };
        release(blob);
        let remove_uuid =
            (blob.wrapper_refs == 0 && blob.reader_refs == 0 && blob.object_url_refs == 0)
                .then(|| blob.uuid.clone());
        if let Some(uuid) = remove_uuid {
            blobs.by_id.remove(&blob_id);
            blobs.ids_by_uuid.remove(&uuid);
        }
    }
}

fn random_uuid() -> String {
    let mut random_bytes = [0_u8; 16];
    getrandom::fill(&mut random_bytes).expect("OS randomness must be available for Blob UUIDs");
    UuidBuilder::from_random_bytes(random_bytes)
        .into_uuid()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_object_url_entries_keep_shared_bytes_and_distinct_creator_keys() {
        let store = BlobStore::<u64, u64, String>::default();
        let blob = store.create_blob(
            Some(1),
            Some(2),
            b"private".to_vec(),
            "text/plain".to_owned(),
        );
        let first_key = "first creator".to_owned();
        let second_key = "second creator".to_owned();
        let first = store
            .create_object_url_with_access_key(Some(2), blob, "null", Some(first_key.clone()))
            .unwrap();
        let second = store
            .create_object_url_with_access_key(Some(3), blob, "null", Some(second_key.clone()))
            .unwrap();
        let captured = store
            .object_url_entry(&format!("{first}#fragment"))
            .unwrap();
        let other = store.object_url_entry(&second).unwrap();
        assert!(Arc::ptr_eq(&captured.bytes, &other.bytes));
        assert_eq!(captured.access_key.as_ref(), Some(&first_key));
        assert_eq!(other.access_key.as_ref(), Some(&second_key));
        assert_eq!(
            store.object_url_bytes_and_type(&first),
            Some((b"private".to_vec(), "text/plain".to_owned()))
        );
        store.release_blob_wrapper_ref(blob);
        assert!(store.revoke_object_url_with_access_key(&first, &first_key));
        assert!(store.revoke_object_url_with_access_key(&second, &second_key));
        assert!(store.object_url_entry(&first).is_none());
        assert!(store.blob_bytes(blob).is_none());
        assert_eq!(&*captured.bytes, b"private");
        assert_eq!(captured.mime_type, "text/plain");
        assert_eq!(captured.access_key.as_ref(), Some(&first_key));
    }

    #[test]
    fn object_url_snapshots_preserve_absent_keys_and_missing_entries() {
        let store = BlobStore::<u64, u64>::default();
        let blob = store.create_blob(None, None, Vec::new(), String::new());
        let url = store.create_object_url(None, blob, "null").unwrap();
        assert!(store.object_url_entry(&url).unwrap().access_key.is_none());
        assert!(store.object_url_entry("blob:null/missing").is_none());
        assert!(store.object_url_entry(&format!("{url}?query")).is_none());
    }

    #[test]
    fn object_url_access_keys_preserve_unauthorized_entries_and_release_authorized_entries() {
        let store = BlobStore::<u64, u64, String>::default();
        let blob = store.create_blob(Some(1), Some(10), b"payload".to_vec(), String::new());
        let first_key = "first URL creator".to_owned();
        let second_key = "second URL creator".to_owned();
        let first = store
            .create_object_url_with_lifetime_and_access_key(
                Some(2),
                Some(101),
                blob,
                "null",
                Some(first_key.clone()),
            )
            .unwrap();
        let second = store
            .create_object_url_with_lifetime_and_access_key(
                Some(3),
                Some(101),
                blob,
                "null",
                Some(second_key.clone()),
            )
            .unwrap();
        let unkeyed = store.create_object_url(Some(1), blob, "null").unwrap();
        store.release_blob_wrapper_ref(blob);

        assert!(!store.revoke_object_url_with_access_key(&first, &second_key));
        assert!(!store.revoke_object_url_with_access_key(&second, &first_key));
        assert!(!store.revoke_object_url_with_access_key(&unkeyed, &first_key));
        assert!(!store.revoke_object_url_with_access_key(&format!("{first}#fragment"), &first_key));
        assert_eq!(store.object_url_body_and_type(&first).unwrap().0, "payload");
        assert!(store.revoke_object_url_with_access_key(&first, &first_key));
        assert!(!store.revoke_object_url_with_access_key(&first, &first_key));
        assert!(store.object_url_body_and_type(&first).is_none());
        assert_eq!(
            store.object_url_body_and_type(&second).unwrap().0,
            "payload"
        );
        assert!(store.revoke_object_url(&unkeyed));
        assert!(store.blob_bytes(blob).is_some());
        assert_eq!(store.cleanup_object_url_lifetime(3, 101), 1);
        assert!(store.blob_bytes(blob).is_none());
        assert!(!store.revoke_object_url_with_access_key(&second, &second_key));
    }

    #[test]
    fn captured_object_url_entries_outlive_revocation_and_creator_cleanup() {
        for cleanup in ["revoke", "owner", "lifetime"] {
            let store = BlobStore::<u64, u64>::default();
            let blob = store.create_blob(
                Some(1),
                Some(10),
                vec![0, 128, 255],
                "application/example".to_owned(),
            );
            let url = store
                .create_object_url_with_lifetime(Some(1), Some(101), blob, "https://example.test")
                .unwrap();
            let entry = store
                .object_url_shared_bytes_and_type(&format!("{url}#fragment"))
                .unwrap();
            let clone = store.object_url_shared_bytes_and_type(&url).unwrap();
            assert!(Arc::ptr_eq(&entry.0, &clone.0));
            let weak = Arc::downgrade(&entry.0);
            store.release_blob_wrapper_ref(blob);
            match cleanup {
                "revoke" => {
                    assert!(store.revoke_object_url(&url));
                }
                "owner" => store.cleanup_owner_resources(1),
                "lifetime" => {
                    assert_eq!(store.cleanup_object_url_lifetime(1, 101), 1);
                }
                _ => unreachable!(),
            }
            assert!(store.object_url_shared_bytes_and_type(&url).is_none());
            assert!(store.blob_bytes(blob).is_none());
            assert_eq!(&*entry.0, &[0, 128, 255]);
            assert_eq!(entry.1, "application/example");
            drop(entry);
            assert!(weak.upgrade().is_some());
            drop(clone);
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn object_url_retains_blob_until_revoked() {
        let store = BlobStore::<u64, u64>::default();
        let blob_id = store.create_blob(
            Some(1),
            Some(10),
            b"hello".to_vec(),
            "text/plain".to_owned(),
        );
        let url = store
            .create_object_url(Some(1), blob_id, "https://example.test")
            .expect("object url");
        let other_url = store
            .create_object_url(Some(1), blob_id, "https://example.test")
            .expect("second object url for the same Blob");
        assert_ne!(url, other_url);
        for object_url in [&url, &other_url] {
            let id = object_url
                .strip_prefix("blob:https://example.test/")
                .unwrap();
            let uuid = uuid::Uuid::parse_str(id).expect("object URLs use UUID identifiers");
            assert_eq!(uuid.get_version_num(), 4);
        }

        store.release_blob_wrapper_ref(blob_id);
        assert_eq!(
            store.object_url_bytes_and_type(&url),
            Some((b"hello".to_vec(), "text/plain".to_owned()))
        );

        assert!(store.revoke_object_url(&url));
        assert!(store.object_url_bytes_and_type(&url).is_none());
        assert_eq!(
            store.object_url_bytes_and_type(&other_url),
            Some((b"hello".to_vec(), "text/plain".to_owned()))
        );

        assert!(store.revoke_object_url(&other_url));
        assert!(store.blob_bytes(blob_id).is_none());
    }

    #[test]
    fn object_url_lookup_ignores_fragment_but_revocation_requires_exact_url() {
        let store = BlobStore::<u64, u64>::default();
        let blob = store.create_blob(Some(1), None, b"hello".to_vec(), "text/plain".to_owned());
        let url = store
            .create_object_url(Some(1), blob, "https://example.test")
            .expect("object URL");
        store.release_blob_wrapper_ref(blob);

        for suffix in ["", "#", "#fragment", "#fragment#tail"] {
            assert_eq!(
                store.object_url_bytes_and_type(&format!("{url}{suffix}")),
                Some((b"hello".to_vec(), "text/plain".to_owned())),
                "lookup should ignore the fragment: {suffix}"
            );
        }
        for suffix in ["?query", "?query#fragment", "/path", "%23fragment"] {
            assert!(
                store
                    .object_url_bytes_and_type(&format!("{url}{suffix}"))
                    .is_none(),
                "lookup must preserve the non-fragment suffix: {suffix}"
            );
        }
        assert!(!store.revoke_object_url(&format!("{url}#fragment")));
        assert!(!store.revoke_object_url(&format!("{url}#")));
        assert_eq!(
            store.object_url_body_and_type(&format!("{url}#fragment")),
            Some(("hello".to_owned(), "text/plain".to_owned()))
        );

        assert!(store.revoke_object_url(&url));
        assert!(
            store
                .object_url_bytes_and_type(&format!("{url}#fragment"))
                .is_none()
        );
        assert!(store.blob_bytes(blob).is_none());
    }

    #[test]
    fn devtools_uuid_is_stable_distinct_and_resolves_bytes() {
        let store = BlobStore::<u64, u64>::default();
        let first = store.create_blob(
            Some(1),
            Some(10),
            b"first".to_vec(),
            "text/plain".to_owned(),
        );
        let second = store.create_blob(
            Some(1),
            Some(10),
            b"second".to_vec(),
            "text/plain".to_owned(),
        );

        let first_uuid = store.blob_uuid(first).expect("first Blob UUID");
        let second_uuid = store.blob_uuid(second).expect("second Blob UUID");
        assert_eq!(store.blob_uuid(first).as_deref(), Some(first_uuid.as_str()));
        assert_ne!(first_uuid, second_uuid);
        assert_eq!(
            uuid::Uuid::parse_str(&first_uuid)
                .expect("UUID syntax")
                .get_version_num(),
            4
        );
        assert_eq!(
            store.blob_bytes_by_uuid_in_partition(&first_uuid, &10),
            Some(b"first".to_vec())
        );
        assert_eq!(
            store.blob_bytes_by_uuid_in_partition(&first_uuid, &11),
            None
        );
        let first_backing = store
            .blob_shared_bytes_by_uuid_in_partition(&first_uuid, &10)
            .expect("first shared backing");
        let repeated_backing = store
            .blob_shared_bytes_by_uuid_in_partition(&first_uuid, &10)
            .expect("repeated shared backing");
        assert!(Arc::ptr_eq(&first_backing, &repeated_backing));
    }

    #[test]
    fn devtools_uuid_stops_resolving_when_blob_is_released() {
        let store = BlobStore::<u64, u64>::default();
        let blob = store.create_blob(Some(1), Some(10), b"released".to_vec(), String::new());
        let uuid = store.blob_uuid(blob).expect("Blob UUID");

        store.release_blob_wrapper_ref(blob);

        assert!(store.blob_uuid(blob).is_none());
        assert!(store.blob_bytes_by_uuid_in_partition(&uuid, &10).is_none());
    }

    #[test]
    fn retiring_owner_preserves_wrappers_readers_and_foreign_urls() {
        let store = BlobStore::<u64, u64>::default();
        let blob = store.create_blob(Some(1), Some(10), b"retained".to_vec(), "text/plain".into());
        let own_url = store
            .create_object_url(Some(1), blob, "https://example.test")
            .unwrap();
        let foreign_url = store
            .create_object_url(Some(2), blob, "https://example.test")
            .unwrap();
        store.retain_blob_reader_ref(blob);
        store.retire_owner_resources(1);
        assert!(store.object_url_bytes_and_type(&own_url).is_none());
        assert_eq!(store.blob_bytes(blob).unwrap(), b"retained");
        store.release_blob_wrapper_ref(blob);
        store.release_blob_reader_ref(blob);
        assert_eq!(
            store.object_url_bytes_and_type(&foreign_url).unwrap().0,
            b"retained"
        );
        assert!(store.revoke_object_url(&foreign_url));
        assert!(store.blob_bytes(blob).is_none());
    }

    #[test]
    fn cleanup_owner_releases_url_references_to_foreign_blobs() {
        let store = BlobStore::<u64, u64>::default();
        let foreign = store.create_blob(Some(2), Some(10), b"foreign".to_vec(), String::new());
        let removed_url = store
            .create_object_url(Some(1), foreign, "https://example.test")
            .unwrap();
        let retained_url = store
            .create_object_url(Some(2), foreign, "https://example.test")
            .unwrap();
        store.release_blob_wrapper_ref(foreign);
        store.cleanup_owner_resources(1);
        assert!(store.object_url_bytes_and_type(&removed_url).is_none());
        assert_eq!(
            store.object_url_bytes_and_type(&retained_url).unwrap().0,
            b"foreign"
        );
        assert!(store.revoke_object_url(&retained_url));
        assert!(store.blob_bytes(foreign).is_none());
    }

    #[test]
    fn cleanup_owner_removes_owned_blobs_and_urls() {
        let store = BlobStore::<u64, u64>::default();
        let owned = store.create_blob(
            Some(1),
            Some(10),
            b"owned".to_vec(),
            "text/plain".to_owned(),
        );
        let other = store.create_blob(
            Some(2),
            Some(10),
            b"other".to_vec(),
            "text/plain".to_owned(),
        );
        let owned_url = store
            .create_object_url(Some(1), owned, "https://example.test")
            .expect("owned url");
        let other_url = store
            .create_object_url(Some(2), other, "https://example.test")
            .expect("other url");

        store.cleanup_owner_resources(1);

        assert!(store.blob_bytes(owned).is_none());
        assert!(store.object_url_bytes_and_type(&owned_url).is_none());
        assert_eq!(
            store.object_url_bytes_and_type(&other_url),
            Some((b"other".to_vec(), "text/plain".to_owned()))
        );
    }

    #[test]
    fn cleanup_object_url_lifetime_revokes_only_matching_urls() {
        let store = BlobStore::<u64, u64>::default();
        let blob = store.create_blob(
            Some(1),
            Some(10),
            b"shared".to_vec(),
            "text/plain".to_owned(),
        );
        let first_url = store
            .create_object_url_with_lifetime(Some(1), Some(101), blob, "https://example.test")
            .expect("first object URL");
        let second_url = store
            .create_object_url_with_lifetime(Some(1), Some(202), blob, "https://example.test")
            .expect("second object URL");
        let other_owner_url = store
            .create_object_url_with_lifetime(Some(2), Some(101), blob, "https://example.test")
            .expect("another owner's URL with the same lifetime identifier");

        assert_eq!(store.cleanup_object_url_lifetime(1, 101), 1);
        assert!(store.object_url_bytes_and_type(&first_url).is_none());
        assert_eq!(
            store.object_url_bytes_and_type(&second_url),
            Some((b"shared".to_vec(), "text/plain".to_owned()))
        );

        store.release_blob_wrapper_ref(blob);
        assert_eq!(store.cleanup_object_url_lifetime(1, 202), 1);
        assert_eq!(
            store.object_url_bytes_and_type(&other_owner_url),
            Some((b"shared".to_vec(), "text/plain".to_owned()))
        );
        assert_eq!(store.cleanup_object_url_lifetime(2, 101), 1);
        assert!(store.blob_bytes(blob).is_none());
    }
}

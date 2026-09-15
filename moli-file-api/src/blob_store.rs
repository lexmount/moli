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
    next_object_url_id: AtomicU64,
}

impl<OwnerId, PartitionId, AccessKey> Default for BlobStore<OwnerId, PartitionId, AccessKey> {
    fn default() -> Self {
        Self {
            blobs: Mutex::default(),
            next_blob_id: AtomicU64::new(1),
            object_urls: Mutex::default(),
            next_object_url_id: AtomicU64::new(1),
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
            let mut random_bytes = [0_u8; 16];
            getrandom::fill(&mut random_bytes)
                .expect("OS randomness must be available for Blob DevTools UUIDs");
            let candidate = UuidBuilder::from_random_bytes(random_bytes)
                .into_uuid()
                .to_string();
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

    /// Create an object URL with its creator environment’s access key.
    /// The key belongs to the URL, independently of the backing Blob.
    pub fn create_object_url_with_access_key(
        &self,
        owner_id: Option<OwnerId>,
        blob_id: BlobId,
        origin: &str,
        access_key: Option<AccessKey>,
    ) -> Option<String> {
        self.retain_blob_object_url_ref(blob_id)?;
        let object_url_id = self
            .next_object_url_id
            .fetch_add(1, Ordering::Relaxed)
            .max(1);
        let object_url = format!("blob:{origin}/{object_url_id}");
        self.object_urls.lock().insert(
            object_url.clone(),
            ObjectUrlState {
                owner_id,
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
        let url = url.split_once('#').map_or(url, |(url, _)| url);
        let blob_id = self
            .object_urls
            .lock()
            .get(url)
            .map(|state| state.blob_id)?;
        let bytes = self.blob_bytes(blob_id)?;
        let mime_type = self.blob_mime_type(blob_id).unwrap_or_default();
        Some((bytes, mime_type))
    }

    /// Return object URL body decoded lossily as text plus MIME type.
    pub fn object_url_body_and_type(&self, url: &str) -> Option<(String, String)> {
        let (bytes, mime_type) = self.object_url_bytes_and_type(url)?;
        Some((String::from_utf8_lossy(&bytes).into_owned(), mime_type))
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

        self.object_urls.lock().retain(|_, state| {
            state.owner_id != Some(owner_id) && !removed_blob_ids.contains(&state.blob_id)
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_url_access_keys_preserve_unauthorized_entries_and_release_authorized_entries() {
        let store = BlobStore::<u64, u64, String>::default();
        let blob = store.create_blob(Some(1), Some(10), b"payload".to_vec(), String::new());
        let first_key = "first URL creator".to_owned();
        let second_key = "second URL creator".to_owned();
        let first = store
            .create_object_url_with_access_key(Some(2), blob, "null", Some(first_key.clone()))
            .unwrap();
        let second = store
            .create_object_url_with_access_key(Some(3), blob, "null", Some(second_key.clone()))
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
        assert!(store.revoke_object_url_with_access_key(&second, &second_key));
        assert!(store.blob_bytes(blob).is_none());
        assert!(!store.revoke_object_url_with_access_key(&second, &second_key));
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

        store.release_blob_wrapper_ref(blob_id);
        assert_eq!(
            store.object_url_bytes_and_type(&url),
            Some((b"hello".to_vec(), "text/plain".to_owned()))
        );

        assert!(store.revoke_object_url(&url));
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
}

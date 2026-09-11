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

#[derive(Clone, Copy, Debug)]
struct ObjectUrlState<OwnerId> {
    owner_id: Option<OwnerId>,
    lifetime_id: Option<u64>,
    blob_id: BlobId,
}

/// Renderer-neutral Blob and object URL backing store.
///
/// The store tracks bytes, MIME type, object URL mappings, and simple reference
/// counts. The embedding layer owns JS wrappers and calls the retain/release
/// hooks from its finalizers.
#[derive(Debug)]
pub struct BlobStore<OwnerId, PartitionId> {
    blobs: Mutex<BlobEntries<OwnerId, PartitionId>>,
    next_blob_id: AtomicU64,
    object_urls: Mutex<HashMap<String, ObjectUrlState<OwnerId>>>,
}

impl<OwnerId, PartitionId> Default for BlobStore<OwnerId, PartitionId> {
    fn default() -> Self {
        Self {
            blobs: Mutex::default(),
            next_blob_id: AtomicU64::new(1),
            object_urls: Mutex::default(),
        }
    }
}

impl<OwnerId, PartitionId> BlobStore<OwnerId, PartitionId>
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
        self.create_object_url_with_lifetime(owner_id, None, blob_id, origin)
    }

    /// Create an object URL tied to a more specific execution-context lifetime.
    pub fn create_object_url_with_lifetime(
        &self,
        owner_id: Option<OwnerId>,
        lifetime_id: Option<u64>,
        blob_id: BlobId,
        origin: &str,
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
            },
        );
        Some(object_url)
    }

    /// Revoke an object URL and release its Blob object-URL reference.
    pub fn revoke_object_url(&self, url: &str) -> bool {
        let Some(state) = self.object_urls.lock().remove(url) else {
            return false;
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
        let blob_id = object_urls.get(url)?.blob_id;
        let blobs = self.blobs.lock();
        let blob = blobs.by_id.get(&blob_id)?;
        Some((blob.bytes.clone(), blob.mime_type.clone()))
    }

    /// Return object URL body decoded lossily as text plus MIME type.
    pub fn object_url_body_and_type(&self, url: &str) -> Option<(String, String)> {
        let (bytes, mime_type) = self.object_url_bytes_and_type(url)?;
        Some((String::from_utf8_lossy(&bytes).into_owned(), mime_type))
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

        let released_blob_ids = {
            let mut object_urls = self.object_urls.lock();
            let mut released_blob_ids = Vec::new();
            object_urls.retain(|_, state| {
                let remove =
                    state.owner_id == Some(owner_id) || removed_blob_ids.contains(&state.blob_id);
                if remove && !removed_blob_ids.contains(&state.blob_id) {
                    released_blob_ids.push(state.blob_id);
                }
                !remove
            });
            released_blob_ids
        };
        for blob_id in released_blob_ids {
            self.release_blob_object_url_ref(blob_id);
        }
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

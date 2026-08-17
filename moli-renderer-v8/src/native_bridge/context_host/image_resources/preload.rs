use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_fetch::FetchCancelHandle;
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::{
    ImageResourceRequestIdentity,
    decode::{ImageDecodeCoordinator, ImageDecodeResult},
    state::{ReadyImageResource, ReadyImageResourceIndex},
};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    network::RendererResourceTaskRunner,
    types::{ImageRequestKey, SharedNavigationResponseResult},
};

const MAX_SCANNED_IMAGE_PRELOADS: usize = 64;
const MAX_SCANNED_IMAGE_PRELOAD_BODY_BYTES: usize = 3 * 1024 * 1024;
const MAX_RETAINED_SCANNED_IMAGE_PRELOAD_BYTES: usize = 15 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScannedImagePreloadKey {
    document_handle: DomHandle,
    request_key: ImageRequestKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScannedImagePreloadIdentity {
    document_handle: DomHandle,
    document_owner: FrameDocumentTaskOwner,
    request_id: u64,
    request_key: ImageRequestKey,
}

pub(crate) enum ScannedImagePreloadAdmission {
    Fetch(SharedScannedImagePreloadLoad),
    Reused,
    Untracked,
}

#[derive(Clone)]
pub(crate) struct SharedScannedImagePreloadLoad {
    inner: Arc<ScannedImagePreloadLoadInner>,
}

struct ScannedImagePreloadLoadInner {
    identity: ScannedImagePreloadIdentity,
    store: Weak<Mutex<ScannedImagePreloadStoreInner>>,
    ready_by_request: ReadyImageResourceIndex,
    decode: ImageDecodeCoordinator,
    cancel_handle: FetchCancelHandle,
    claimed: AtomicBool,
    state: Mutex<ScannedImagePreloadLoadState>,
    notify: Notify,
}

#[derive(Default)]
struct ScannedImagePreloadLoadState {
    outcome: Option<Arc<ScannedImagePreloadOutcome>>,
}

pub(crate) struct ScannedImagePreloadOutcome {
    network_result: SharedNavigationResponseResult,
    encoded: Option<moli_parkable_image::ParkableImage>,
    _resource: Option<Arc<ReadyImageResource>>,
    _retained_body_permit: Option<ScannedImagePreloadBodyPermit>,
}

impl ScannedImagePreloadOutcome {
    pub(crate) fn network_result(&self) -> &SharedNavigationResponseResult {
        &self.network_result
    }

    pub(crate) fn encoded(&self) -> Option<moli_parkable_image::ParkableImage> {
        self.encoded.clone()
    }
}

struct ScannedImagePreloadBodyPermit {
    store: Weak<Mutex<ScannedImagePreloadStoreInner>>,
    bytes: usize,
}

impl Drop for ScannedImagePreloadBodyPermit {
    fn drop(&mut self) {
        let Some(store) = self.store.upgrade() else {
            return;
        };
        let mut store = store.lock();
        store.retained_body_bytes = store.retained_body_bytes.saturating_sub(self.bytes);
    }
}

#[derive(Clone)]
pub(super) struct ScannedImagePreloadStore {
    inner: Arc<Mutex<ScannedImagePreloadStoreInner>>,
    ready_by_request: ReadyImageResourceIndex,
    decode: ImageDecodeCoordinator,
}

#[derive(Default)]
struct ScannedImagePreloadStoreInner {
    next_request_id: u64,
    retained_body_bytes: usize,
    entries: HashMap<ScannedImagePreloadKey, ScannedImagePreloadEntry>,
}

struct ScannedImagePreloadEntry {
    identity: ScannedImagePreloadIdentity,
    load: SharedScannedImagePreloadLoad,
}

impl Drop for ScannedImagePreloadStoreInner {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            entry.load.cancel();
        }
    }
}

impl ScannedImagePreloadStore {
    pub(super) fn new(
        ready_by_request: ReadyImageResourceIndex,
        decode: ImageDecodeCoordinator,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ScannedImagePreloadStoreInner::default())),
            ready_by_request,
            decode,
        }
    }

    pub(super) fn admit(
        &self,
        document_handle: DomHandle,
        document_owner: FrameDocumentTaskOwner,
        request_key: ImageRequestKey,
    ) -> ScannedImagePreloadAdmission {
        let key = ScannedImagePreloadKey {
            document_handle,
            request_key: request_key.clone(),
        };
        let mut inner = self.inner.lock();
        if inner.entries.get(&key).is_some_and(|entry| {
            entry.identity.document_owner == document_owner
                && !entry.load.inner.cancel_handle.is_cancelled()
        }) {
            return ScannedImagePreloadAdmission::Reused;
        }
        if !inner.entries.contains_key(&key) && inner.entries.len() >= MAX_SCANNED_IMAGE_PRELOADS {
            return ScannedImagePreloadAdmission::Untracked;
        }
        let Some(request_id) = inner.next_request_id.checked_add(1) else {
            return ScannedImagePreloadAdmission::Untracked;
        };
        inner.next_request_id = request_id;
        if let Some(stale) = inner.entries.remove(&key) {
            stale.load.cancel();
        }
        let identity = ScannedImagePreloadIdentity {
            document_handle,
            document_owner,
            request_id,
            request_key,
        };
        let load = SharedScannedImagePreloadLoad {
            inner: Arc::new(ScannedImagePreloadLoadInner {
                identity: identity.clone(),
                store: Arc::downgrade(&self.inner),
                ready_by_request: self.ready_by_request.clone(),
                decode: self.decode.clone(),
                cancel_handle: FetchCancelHandle::new(),
                claimed: AtomicBool::new(false),
                state: Mutex::new(ScannedImagePreloadLoadState::default()),
                notify: Notify::new(),
            }),
        };
        inner.entries.insert(
            key,
            ScannedImagePreloadEntry {
                identity,
                load: load.clone(),
            },
        );
        ScannedImagePreloadAdmission::Fetch(load)
    }

    pub(super) fn contains(&self, identity: &ImageResourceRequestIdentity) -> bool {
        let key = ScannedImagePreloadKey {
            document_handle: identity.document_handle,
            request_key: identity.request_key.clone(),
        };
        self.inner.lock().entries.get(&key).is_some_and(|entry| {
            entry.identity.document_owner == identity.document_owner
                && !entry.load.inner.cancel_handle.is_cancelled()
        })
    }

    pub(super) fn claim(
        &self,
        identity: &ImageResourceRequestIdentity,
    ) -> Option<SharedScannedImagePreloadLoad> {
        let key = ScannedImagePreloadKey {
            document_handle: identity.document_handle,
            request_key: identity.request_key.clone(),
        };
        let mut inner = self.inner.lock();
        let entry = inner.entries.get(&key)?;
        if entry.identity.document_owner != identity.document_owner {
            return None;
        }
        let entry = inner
            .entries
            .remove(&key)
            .expect("checked scanned image preload entry must remain present");
        entry.load.inner.claimed.store(true, Ordering::Release);
        Some(entry.load)
    }

    pub(super) fn retire_document(&self, document_handle: DomHandle) -> usize {
        let mut inner = self.inner.lock();
        let retired = inner
            .entries
            .extract_if(|key, _| key.document_handle == document_handle)
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>();
        let count = retired.len();
        drop(inner);
        for entry in retired {
            entry.load.cancel();
        }
        count
    }

    #[cfg(test)]
    pub(super) fn entry_count(&self) -> usize {
        self.inner.lock().entries.len()
    }
}

impl SharedScannedImagePreloadLoad {
    pub(crate) fn cancel_handle(&self) -> FetchCancelHandle {
        self.inner.cancel_handle.clone()
    }

    pub(crate) fn cancel(&self) {
        self.inner.cancel_handle.cancel();
    }

    pub(crate) fn request_key(&self) -> &ImageRequestKey {
        &self.inner.identity.request_key
    }

    pub(crate) fn finish_network_result(
        &self,
        runner: RendererResourceTaskRunner,
        network_result: SharedNavigationResponseResult,
        encoded: Option<moli_parkable_image::ParkableImage>,
        response_is_decode_eligible: bool,
    ) {
        if self.try_outcome().is_some() {
            return;
        }
        let encoded_bytes = encoded.as_ref().map_or(0, |encoded| encoded.len());
        let retained_body_permit = if encoded_bytes == 0 {
            None
        } else {
            match self.reserve_retained_body(encoded_bytes) {
                Some(permit) => permit,
                None if self.inner.claimed.load(Ordering::Acquire) => None,
                None => {
                    if self.abandon_unclaimed() {
                        return;
                    }
                    None
                }
            }
        };

        let descriptor = response_is_decode_eligible
            .then(|| {
                network_result
                    .as_ref()
                    .as_ref()
                    .ok()
                    .zip(encoded.as_ref())
                    .and_then(|(response, encoded)| {
                        crate::network_host::image_response_descriptor_from_parkable(
                            response, encoded,
                        )
                    })
            })
            .flatten();
        let Some(descriptor) = descriptor else {
            self.finish(ScannedImagePreloadOutcome {
                network_result,
                encoded,
                _resource: None,
                _retained_body_permit: retained_body_permit,
            });
            return;
        };
        let encoded = encoded.expect("a decoded preload descriptor requires encoded bytes");
        let load = self.clone();
        let completion_state = Arc::new(Mutex::new(Some((
            network_result,
            encoded.clone(),
            retained_body_permit,
        ))));
        let completion_state_for_decode = completion_state.clone();
        let result = self.inner.decode.submit_preload(
            runner,
            descriptor.decode_metadata,
            encoded,
            move |decode_result| {
                let Some((network_result, encoded, retained_body_permit)) =
                    completion_state_for_decode.lock().take()
                else {
                    return;
                };
                match decode_result {
                    ImageDecodeResult::Ready(ready) => {
                        let (pixels, svg) = match ready.content {
                            super::decode::DecodedImageContent::Raster(image) => {
                                (Some(image), None)
                            }
                            super::decode::DecodedImageContent::Svg(image) => (None, Some(image)),
                        };
                        let resource = Arc::new(ReadyImageResource {
                            descriptor,
                            density: load.inner.identity.request_key.density(),
                            pixels,
                            svg,
                            encoded: Some(ready.encoded),
                            _decoded_bytes_permit: Some(ready.decoded_bytes_permit),
                        });
                        load.inner
                            .ready_by_request
                            .insert(load.inner.identity.request_key.clone(), &resource);
                        load.finish(ScannedImagePreloadOutcome {
                            network_result,
                            encoded: Some(encoded),
                            _resource: Some(resource),
                            _retained_body_permit: retained_body_permit,
                        });
                    }
                    ImageDecodeResult::Failed(error) => {
                        tracing::debug!(
                            document = load.inner.identity.document_handle.index(),
                            request_id = load.inner.identity.request_id,
                            url = load.inner.identity.request_key.url(),
                            %error,
                            "scanned image preload decode failed"
                        );
                        load.finish(ScannedImagePreloadOutcome {
                            network_result,
                            encoded: Some(encoded),
                            _resource: None,
                            _retained_body_permit: retained_body_permit,
                        });
                    }
                }
            },
        );
        if result.is_err() {
            let (network_result, encoded, retained_body_permit) = completion_state
                .lock()
                .take()
                .expect("rejected preload decode must retain its completion state");
            self.finish(ScannedImagePreloadOutcome {
                network_result,
                encoded: Some(encoded),
                _resource: None,
                _retained_body_permit: retained_body_permit,
            });
        }
    }

    pub(crate) fn try_outcome(&self) -> Option<Arc<ScannedImagePreloadOutcome>> {
        self.inner.state.lock().outcome.clone()
    }

    pub(crate) async fn wait_outcome(&self) -> Arc<ScannedImagePreloadOutcome> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(outcome) = self.try_outcome() {
                return outcome;
            }
            notified.await;
        }
    }

    fn finish(&self, outcome: ScannedImagePreloadOutcome) {
        let stored = {
            let mut state = self.inner.state.lock();
            if state.outcome.is_some() {
                false
            } else {
                state.outcome = Some(Arc::new(outcome));
                true
            }
        };
        if stored {
            self.inner.notify.notify_waiters();
        }
    }

    fn reserve_retained_body(&self, bytes: usize) -> Option<Option<ScannedImagePreloadBodyPermit>> {
        if bytes > MAX_SCANNED_IMAGE_PRELOAD_BODY_BYTES {
            return None;
        }
        let store = self.inner.store.upgrade()?;
        let mut inner = store.lock();
        let retained_body_bytes = inner.retained_body_bytes.checked_add(bytes)?;
        if retained_body_bytes > MAX_RETAINED_SCANNED_IMAGE_PRELOAD_BYTES {
            return None;
        }
        inner.retained_body_bytes = retained_body_bytes;
        Some(Some(ScannedImagePreloadBodyPermit {
            store: Arc::downgrade(&store),
            bytes,
        }))
    }

    /// Returns true only when no element can still claim this load.
    fn abandon_unclaimed(&self) -> bool {
        if self.inner.claimed.load(Ordering::Acquire) {
            return false;
        }
        let Some(store) = self.inner.store.upgrade() else {
            return true;
        };
        let key = ScannedImagePreloadKey {
            document_handle: self.inner.identity.document_handle,
            request_key: self.inner.identity.request_key.clone(),
        };
        let mut store = store.lock();
        if self.inner.claimed.load(Ordering::Acquire) {
            return false;
        }
        if store
            .entries
            .get(&key)
            .is_some_and(|entry| entry.identity.request_id == self.inner.identity.request_id)
        {
            store.entries.remove(&key);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId},
        native_bridge::{
            ImageLoadEventId, context_host::image_resources::state::ImageResourceStore,
        },
        types::ImageRequestCorsMode,
    };

    fn owner(document: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(1),
            LocalWindowId(2),
            DocumentId(document),
        )
    }

    fn request_key(url: &str) -> ImageRequestKey {
        ImageRequestKey::with_density(url.to_owned(), ImageRequestCorsMode::NoCors, 1.0)
    }

    #[test]
    fn exact_document_preload_is_claimed_once_and_keeps_its_terminal_result() {
        let ready = ReadyImageResourceIndex::default();
        let store = ScannedImagePreloadStore::new(ready, ImageDecodeCoordinator::default());
        let document = DomHandle::new(10);
        let element = DomHandle::new(11);
        let key = request_key("https://example.test/hero.png");
        let ScannedImagePreloadAdmission::Fetch(load) =
            store.admit(document, owner(1), key.clone())
        else {
            panic!("the first exact image request must start a preload");
        };
        assert!(matches!(
            store.admit(document, owner(1), key.clone()),
            ScannedImagePreloadAdmission::Reused
        ));

        let identity = ImageResourceRequestIdentity {
            element,
            sequence: ImageLoadEventId::new(1),
            document_handle: document,
            document_owner: owner(1),
            request_key: key,
        };
        assert!(store.contains(&identity));
        let claimed = store
            .claim(&identity)
            .expect("the matching element must claim the scanner request");
        assert_eq!(store.entry_count(), 0);
        assert!(store.claim(&identity).is_none());
        assert!(Arc::ptr_eq(&claimed.inner, &load.inner));

        claimed.finish_network_result(
            RendererResourceTaskRunner::for_test(),
            Arc::new(Err("network failed".to_owned())),
            None,
            false,
        );
        let outcome = claimed
            .try_outcome()
            .expect("a failed physical request must still settle its claimant");
        assert!(matches!(
            outcome.network_result().as_ref(),
            Err(error) if error == "network failed"
        ));
    }

    #[test]
    fn replacing_or_retiring_a_document_preload_cancels_its_physical_request() {
        let ready = ReadyImageResourceIndex::default();
        let store = ScannedImagePreloadStore::new(ready, ImageDecodeCoordinator::default());
        let document = DomHandle::new(20);
        let key = request_key("https://example.test/replace.png");
        let ScannedImagePreloadAdmission::Fetch(stale) =
            store.admit(document, owner(1), key.clone())
        else {
            panic!("the first request must be admitted");
        };
        let ScannedImagePreloadAdmission::Fetch(current) = store.admit(document, owner(2), key)
        else {
            panic!("a replacement Document owner must receive a new request");
        };
        assert!(stale.cancel_handle().is_cancelled());
        assert!(!current.cancel_handle().is_cancelled());
        assert_eq!(store.retire_document(document), 1);
        assert!(current.cancel_handle().is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decoded_preload_is_ready_when_the_exact_element_claims_it() {
        let mut resources = ImageResourceStore::default();
        let document = DomHandle::new(30);
        let element = DomHandle::new(31);
        let url = url::Url::parse("https://example.test/ready.png").expect("image URL");
        let key = request_key(url.as_str());
        let ScannedImagePreloadAdmission::Fetch(load) =
            resources.scanned.admit(document, owner(1), key.clone())
        else {
            panic!("the exact image request must start a preload");
        };
        let pixels =
            moli_image::RgbaImage::try_new(1, 1, vec![1, 2, 3, 255]).expect("valid image pixels");
        let encoded = moli_image::encode_png(&pixels).expect("valid PNG");
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            url,
            200,
            vec![("Content-Type".to_owned(), "image/png".to_owned())],
            String::new(),
        );
        let encoded =
            moli_parkable_image::ParkableImageManager::default().from_frozen_bytes(encoded.bytes);

        load.finish_network_result(
            RendererResourceTaskRunner::for_test(),
            Arc::new(Ok(response)),
            Some(encoded),
            true,
        );
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), load.wait_outcome())
            .await
            .expect("preload decode should finish before the deadline");
        assert!(outcome.network_result().as_ref().is_ok());

        let identity = ImageResourceRequestIdentity {
            element,
            sequence: ImageLoadEventId::new(1),
            document_handle: document,
            document_owner: owner(1),
            request_key: key,
        };
        resources.begin(identity.clone());
        let claimed = resources
            .claim_scanned_preload(element)
            .expect("the exact element should claim the decoded preload");
        drop(claimed);
        drop(outcome);
        drop(load);
        assert!(resources.complete_shared_ready(&identity));
        let ready = resources
            .ready_for_layout(element)
            .expect("the exact element should bind the decoded preload");
        assert_eq!((ready.intrinsic_width, ready.intrinsic_height), (1.0, 1.0));
        assert_eq!(
            ready
                .pixels
                .expect("decoded raster pixels")
                .as_ref()
                .as_ref(),
            &[1, 2, 3, 255]
        );
    }

    #[test]
    fn claimed_oversize_response_still_completes_without_a_body_permit() {
        let ready = ReadyImageResourceIndex::default();
        let store = ScannedImagePreloadStore::new(ready, ImageDecodeCoordinator::default());
        let document = DomHandle::new(40);
        let element = DomHandle::new(41);
        let url = url::Url::parse("https://example.test/oversize.png").expect("image URL");
        let key = request_key(url.as_str());
        let ScannedImagePreloadAdmission::Fetch(load) =
            store.admit(document, owner(1), key.clone())
        else {
            panic!("the exact image request must start a preload");
        };
        let identity = ImageResourceRequestIdentity {
            element,
            sequence: ImageLoadEventId::new(1),
            document_handle: document,
            document_owner: owner(1),
            request_key: key,
        };
        assert!(store.claim(&identity).is_some());
        let body_bytes = vec![0; MAX_SCANNED_IMAGE_PRELOAD_BODY_BYTES + 1];
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            url,
            200,
            vec![("Content-Type".to_owned(), "image/png".to_owned())],
            String::new(),
        );
        let encoded =
            moli_parkable_image::ParkableImageManager::default().from_frozen_bytes(body_bytes);

        load.finish_network_result(
            RendererResourceTaskRunner::for_test(),
            Arc::new(Ok(response)),
            Some(encoded),
            false,
        );
        let outcome = load
            .try_outcome()
            .expect("a claimed oversize response must settle its waiter");
        assert!(outcome.network_result().as_ref().is_ok());
        assert!(outcome._retained_body_permit.is_none());
    }
}

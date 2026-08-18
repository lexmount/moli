use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;

use super::{
    ImageResponseDescriptor,
    decode::{DecodedImageContent, ReadyDecodedImage},
    state::{
        ReadyImageForLayout, ReadyImageResource, ReadyImageResourceIndex, intrinsic_dimensions,
    },
};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    types::{ImageRequestCorsMode, ImageRequestKey},
};

/// Upper bound for distinct CSS image resources retained by one renderer host.
///
/// Decoded bytes are independently bounded by the shared image budget. This
/// limit also bounds pending/failed URL bookkeeping under hostile CSSOM churn.
const MAX_CSS_IMAGE_RESOURCE_SLOTS: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CssImageResourceRequestIdentity {
    pub(crate) document_handle: DomHandle,
    pub(crate) document_owner: FrameDocumentTaskOwner,
    pub(crate) request_id: u64,
    pub(crate) request_key: ImageRequestKey,
}

pub(crate) enum CssImageResourceAdmission {
    Fetch(CssImageResourceRequestIdentity),
    Reused,
    /// Preserve the existing network-observable fetch when the bounded
    /// renderer sidecar cannot retain another URL.
    Untracked,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CssImageResourceKey {
    document_handle: DomHandle,
    request_key: ImageRequestKey,
}

#[derive(Clone)]
pub(super) struct CssImageResourceStore {
    inner: Arc<Mutex<CssImageResourceStoreInner>>,
    ready_by_request: ReadyImageResourceIndex,
    #[cfg(test)]
    completion_notify: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
struct CssImageResourceStoreInner {
    next_request_id: u64,
    slots: HashMap<CssImageResourceKey, CssImageResourceSlot>,
}

struct CssImageResourceSlot {
    identity: CssImageResourceRequestIdentity,
    state: CssImageResourceState,
}

enum CssImageResourceState {
    Pending,
    DecodeQueued(ImageResponseDescriptor),
    Ready(Arc<ReadyImageResource>),
    Failed,
}

impl CssImageResourceStore {
    pub(super) fn new(ready_by_request: ReadyImageResourceIndex) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CssImageResourceStoreInner::default())),
            ready_by_request,
            #[cfg(test)]
            completion_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Admits one exact Document/URL request.
    ///
    pub(super) fn admit(
        &self,
        document_handle: DomHandle,
        document_owner: FrameDocumentTaskOwner,
        resolved_url: String,
    ) -> CssImageResourceAdmission {
        let request_key =
            ImageRequestKey::with_density(resolved_url, ImageRequestCorsMode::NoCors, 1.0);
        let key = CssImageResourceKey {
            document_handle,
            request_key: request_key.clone(),
        };
        let mut inner = self.inner.lock();
        if inner
            .slots
            .get(&key)
            .is_some_and(|slot| slot.identity.document_owner == document_owner)
        {
            return CssImageResourceAdmission::Reused;
        }
        if !inner.slots.contains_key(&key) && inner.slots.len() >= MAX_CSS_IMAGE_RESOURCE_SLOTS {
            return CssImageResourceAdmission::Untracked;
        }
        let Some(request_id) = inner.next_request_id.checked_add(1) else {
            return CssImageResourceAdmission::Untracked;
        };
        inner.next_request_id = request_id;
        let identity = CssImageResourceRequestIdentity {
            document_handle,
            document_owner,
            request_id,
            request_key,
        };
        let shared_ready = self.ready_by_request.get_decoded(&identity.request_key);
        let reused = shared_ready.is_some();
        let state = shared_ready
            .map(CssImageResourceState::Ready)
            .unwrap_or(CssImageResourceState::Pending);
        inner.slots.insert(
            key,
            CssImageResourceSlot {
                identity: identity.clone(),
                state,
            },
        );
        if reused {
            CssImageResourceAdmission::Reused
        } else {
            CssImageResourceAdmission::Fetch(identity)
        }
    }

    pub(super) fn mark_decode_queued(
        &self,
        identity: &CssImageResourceRequestIdentity,
        descriptor: ImageResponseDescriptor,
    ) -> bool {
        let mut inner = self.inner.lock();
        let Some(slot) = current_slot_mut(&mut inner, identity) else {
            return false;
        };
        if !matches!(slot.state, CssImageResourceState::Pending) {
            return false;
        }
        slot.state = CssImageResourceState::DecodeQueued(descriptor);
        true
    }

    pub(super) fn complete_decode(
        &self,
        identity: &CssImageResourceRequestIdentity,
        ready: ReadyDecodedImage,
    ) -> bool {
        let mut inner = self.inner.lock();
        let Some(slot) = current_slot_mut(&mut inner, identity) else {
            return false;
        };
        let descriptor = match &slot.state {
            CssImageResourceState::DecodeQueued(descriptor) => *descriptor,
            _ => return false,
        };
        let (pixels, svg) = match ready.content {
            DecodedImageContent::Raster(image) => (Some(image), None),
            DecodedImageContent::Svg(image) => (None, Some(image)),
        };
        let resource = Arc::new(ReadyImageResource {
            descriptor,
            density: identity.request_key.density(),
            pixels,
            svg,
            _decoded_bytes_permit: Some(ready.decoded_bytes_permit),
        });
        self.ready_by_request
            .insert(identity.request_key.clone(), &resource);
        slot.state = CssImageResourceState::Ready(resource);
        #[cfg(test)]
        self.completion_notify.notify_waiters();
        true
    }

    pub(super) fn fail(&self, identity: &CssImageResourceRequestIdentity) -> bool {
        let mut inner = self.inner.lock();
        let Some(slot) = current_slot_mut(&mut inner, identity) else {
            return false;
        };
        if !matches!(
            slot.state,
            CssImageResourceState::Pending | CssImageResourceState::DecodeQueued(_)
        ) {
            return false;
        }
        slot.state = CssImageResourceState::Failed;
        #[cfg(test)]
        self.completion_notify.notify_waiters();
        true
    }

    pub(super) fn ready_for_layout(
        &self,
        document_handle: DomHandle,
        resolved_url: &str,
    ) -> Option<ReadyImageForLayout> {
        let key = CssImageResourceKey {
            document_handle,
            request_key: ImageRequestKey::with_density(
                resolved_url.to_owned(),
                ImageRequestCorsMode::NoCors,
                1.0,
            ),
        };
        let inner = self.inner.lock();
        let slot = inner.slots.get(&key)?;
        let CssImageResourceState::Ready(resource) = &slot.state else {
            return None;
        };
        let (intrinsic_width, intrinsic_height) = intrinsic_dimensions(resource);
        Some(ReadyImageForLayout {
            intrinsic_width,
            intrinsic_height,
            pixels: resource.pixels.clone(),
            svg: resource.svg.clone(),
        })
    }

    pub(super) fn retire_document(&self, document_handle: DomHandle) -> usize {
        let mut inner = self.inner.lock();
        let before = inner.slots.len();
        inner
            .slots
            .retain(|key, _| key.document_handle != document_handle);
        let removed = before - inner.slots.len();
        drop(inner);
        self.ready_by_request.prune_dead();
        removed
    }

    #[cfg(test)]
    pub(super) fn observability_for_test(&self) -> (usize, usize, usize, usize, Vec<String>) {
        let inner = self.inner.lock();
        let mut pending = 0;
        let mut decode_queued = 0;
        let mut ready = 0;
        let mut failed = 0;
        let mut urls = Vec::with_capacity(inner.slots.len());
        for (key, slot) in &inner.slots {
            urls.push(key.request_key.url().to_owned());
            match &slot.state {
                CssImageResourceState::Pending => pending += 1,
                CssImageResourceState::DecodeQueued(_) => decode_queued += 1,
                CssImageResourceState::Ready(_) => ready += 1,
                CssImageResourceState::Failed => failed += 1,
            }
        }
        urls.sort();
        (pending, decode_queued, ready, failed, urls)
    }

    #[cfg(test)]
    pub(super) fn completion_notify_for_test(&self) -> Arc<tokio::sync::Notify> {
        self.completion_notify.clone()
    }
}

fn current_slot_mut<'a>(
    inner: &'a mut CssImageResourceStoreInner,
    identity: &CssImageResourceRequestIdentity,
) -> Option<&'a mut CssImageResourceSlot> {
    let key = CssImageResourceKey {
        document_handle: identity.document_handle,
        request_key: identity.request_key.clone(),
    };
    inner
        .slots
        .get_mut(&key)
        .filter(|slot| slot.identity == *identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

    fn owner(document: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(1),
            LocalWindowId(2),
            DocumentId(document),
        )
    }

    fn ready_red_pixel() -> ReadyDecodedImage {
        let budget = super::super::budget::SharedImageResourceBudget::default();
        ReadyDecodedImage {
            content: DecodedImageContent::Raster(Arc::new(
                moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 255]).unwrap(),
            )),
            decoded_bytes_permit: budget.reserve_decoded(4).unwrap(),
        }
    }

    #[test]
    fn exact_document_request_identity_rejects_stale_decode_and_shares_ready_content() {
        let ready_index = ReadyImageResourceIndex::default();
        let store = CssImageResourceStore::new(ready_index.clone());
        let first_document = DomHandle::new(10);
        let second_document = DomHandle::new(20);
        let url = "https://example.test/shared.png";
        let descriptor = ImageResponseDescriptor::raster(moli_image::RasterImageMetadata {
            format: moli_image::RasterImageFormat::Png,
            width: 1,
            height: 1,
        });

        let CssImageResourceAdmission::Fetch(first) =
            store.admit(first_document, owner(1), url.to_owned())
        else {
            panic!("the first exact request must fetch");
        };
        assert!(store.mark_decode_queued(&first, descriptor));

        let CssImageResourceAdmission::Fetch(replacement) =
            store.admit(first_document, owner(2), url.to_owned())
        else {
            panic!("a replacement Document owner must receive a new request identity");
        };
        assert!(
            !store.complete_decode(&first, ready_red_pixel()),
            "the retired owner cannot commit into the replacement slot"
        );
        assert!(store.mark_decode_queued(&replacement, descriptor));
        assert!(store.complete_decode(&replacement, ready_red_pixel()));
        assert!(
            !store.fail(&replacement),
            "a duplicate failure terminal must not replace ready content"
        );
        assert_eq!(
            store
                .ready_for_layout(first_document, url)
                .expect("replacement ready resource")
                .pixels
                .expect("decoded raster")
                .rgba,
            [255, 0, 0, 255]
        );

        assert!(matches!(
            store.admit(second_document, owner(3), url.to_owned()),
            CssImageResourceAdmission::Reused
        ));
        assert!(
            store.ready_for_layout(second_document, url).is_some(),
            "another live Document may share the immutable decoded resource"
        );
        assert_eq!(store.retire_document(first_document), 1);
        assert!(store.ready_for_layout(second_document, url).is_some());
        assert_eq!(store.retire_document(second_document), 1);

        assert!(matches!(
            store.admit(DomHandle::new(30), owner(4), url.to_owned()),
            CssImageResourceAdmission::Fetch(_)
        ));

        let metadata_only_url = "https://example.test/metadata-only.png";
        let metadata_only_key = ImageRequestKey::with_density(
            metadata_only_url.to_owned(),
            ImageRequestCorsMode::NoCors,
            1.0,
        );
        let metadata_only = Arc::new(ReadyImageResource {
            descriptor,
            density: 1.0,
            pixels: None,
            svg: None,
            _decoded_bytes_permit: None,
        });
        ready_index.insert(metadata_only_key, &metadata_only);
        assert!(matches!(
            store.admit(DomHandle::new(40), owner(5), metadata_only_url.to_owned()),
            CssImageResourceAdmission::Fetch(_)
        ));
    }
}

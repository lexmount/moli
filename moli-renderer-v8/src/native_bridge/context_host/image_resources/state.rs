use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;

use super::{
    ImageResourceRequestIdentity, ImageResponseDescriptor,
    budget::ImageDecodedBytesPermit,
    css::CssImageResourceStore,
    decode::{DecodedImageContent, ImageDecodeCoordinator, ReadyDecodedImage},
    preload::{ScannedImagePreloadStore, SharedScannedImagePreloadLoad},
};
use crate::document_runtime::DomHandle;
use crate::types::ImageRequestKey;

#[derive(Clone)]
pub(crate) struct ReadyImageForLayout {
    pub(crate) intrinsic_width: f32,
    pub(crate) intrinsic_height: f32,
    pub(crate) pixels: Option<Arc<moli_image::RgbaImage>>,
    pub(crate) svg: Option<Arc<moli_image::SvgImage>>,
}

pub(in crate::native_bridge::context_host) struct ImageResourceStore {
    slots: HashMap<DomHandle, ImageResourceSlot>,
    ready_by_request: ReadyImageResourceIndex,
    pub(super) css: CssImageResourceStore,
    pub(super) scanned: ScannedImagePreloadStore,
    pub(super) decode: ImageDecodeCoordinator,
}

struct ImageResourceSlot {
    identity: ImageResourceRequestIdentity,
    state: ImageResourceState,
    scanned_preload: Option<SharedScannedImagePreloadLoad>,
}

enum ImageResourceState {
    Pending,
    DecodeQueued(ImageResponseDescriptor),
    Ready(Arc<ReadyImageResource>),
    Failed,
}

pub(super) struct ReadyImageResource {
    pub(super) descriptor: ImageResponseDescriptor,
    pub(super) density: f64,
    pub(super) pixels: Option<Arc<moli_image::RgbaImage>>,
    pub(super) svg: Option<Arc<moli_image::SvgImage>>,
    pub(super) _decoded_bytes_permit: Option<ImageDecodedBytesPermit>,
}

/// Weak exact-request index shared by HTML image slots and CSS image slots.
///
/// The index owns no decoded resource. It only lets independent consumers of
/// the same URL/CORS/density tuple reuse an immutable resource while at least
/// one document-owned slot still retains it.
#[derive(Clone, Default)]
pub(super) struct ReadyImageResourceIndex {
    inner: Arc<Mutex<HashMap<ImageRequestKey, Weak<ReadyImageResource>>>>,
}

impl ReadyImageResourceIndex {
    pub(super) fn get(&self, request: &ImageRequestKey) -> Option<Arc<ReadyImageResource>> {
        self.inner.lock().get(request).and_then(Weak::upgrade)
    }

    /// CSS paint may only reuse content that has actually been decoded.
    /// An HTML image slot can be metadata-ready under `Mock` layout; treating
    /// that entry as paint-ready would suppress the CSS consumer's fetch while
    /// providing neither pixels nor a vector tree.
    pub(super) fn get_decoded(&self, request: &ImageRequestKey) -> Option<Arc<ReadyImageResource>> {
        self.get(request)
            .filter(|resource| resource.pixels.is_some() || resource.svg.is_some())
    }

    pub(super) fn insert(&self, request: ImageRequestKey, resource: &Arc<ReadyImageResource>) {
        self.inner.lock().insert(request, Arc::downgrade(resource));
    }

    pub(super) fn contains_live(&self, request: &ImageRequestKey) -> bool {
        self.inner
            .lock()
            .get(request)
            .is_some_and(|resource| resource.strong_count() > 0)
    }

    pub(super) fn prune_dead(&self) {
        self.inner
            .lock()
            .retain(|_, resource| resource.strong_count() > 0);
    }
}

impl Default for ImageResourceStore {
    fn default() -> Self {
        let ready_by_request = ReadyImageResourceIndex::default();
        let decode = ImageDecodeCoordinator::default();
        Self {
            slots: HashMap::new(),
            css: CssImageResourceStore::new(ready_by_request.clone()),
            scanned: ScannedImagePreloadStore::new(ready_by_request.clone(), decode.clone()),
            ready_by_request,
            decode,
        }
    }
}

impl ImageResourceStore {
    pub(super) fn begin(&mut self, identity: ImageResourceRequestIdentity) {
        let ready = self.ready_by_request.get(&identity.request_key);
        self.slots.insert(
            identity.element,
            ImageResourceSlot {
                identity,
                state: ready.map_or(ImageResourceState::Pending, ImageResourceState::Ready),
                scanned_preload: None,
            },
        );
    }

    pub(super) fn identity(&self, element: DomHandle) -> Option<&ImageResourceRequestIdentity> {
        self.slots.get(&element).map(|slot| &slot.identity)
    }

    pub(super) fn claim_scanned_preload(
        &mut self,
        element: DomHandle,
    ) -> Option<SharedScannedImagePreloadLoad> {
        let identity = self.identity(element)?.clone();
        let load = self.scanned.claim(&identity)?;
        let slot = self
            .slots
            .get_mut(&element)
            .expect("a claimed image preload must retain its exact element slot");
        slot.scanned_preload = Some(load.clone());
        Some(load)
    }

    pub(super) fn mark_decode_queued(
        &mut self,
        identity: &ImageResourceRequestIdentity,
        descriptor: ImageResponseDescriptor,
    ) -> bool {
        let Some(slot) = self.slots.get_mut(&identity.element) else {
            return false;
        };
        if slot.identity != *identity || !matches!(slot.state, ImageResourceState::Pending) {
            return false;
        }
        slot.scanned_preload = None;
        slot.state = ImageResourceState::DecodeQueued(descriptor);
        true
    }

    pub(super) fn complete_metadata(
        &mut self,
        identity: &ImageResourceRequestIdentity,
        descriptor: ImageResponseDescriptor,
    ) -> bool {
        self.complete_ready(identity, descriptor, None, None, None)
    }

    pub(super) fn complete_decode(
        &mut self,
        identity: &ImageResourceRequestIdentity,
        ready: ReadyDecodedImage,
    ) -> bool {
        let descriptor = match self.slots.get(&identity.element) {
            Some(ImageResourceSlot {
                identity: current,
                state: ImageResourceState::DecodeQueued(descriptor),
                ..
            }) if current == identity => *descriptor,
            _ => return false,
        };
        let (pixels, svg) = match ready.content {
            DecodedImageContent::Raster(image) => (Some(image), None),
            DecodedImageContent::Svg(image) => (None, Some(image)),
        };
        self.complete_ready(
            identity,
            descriptor,
            pixels,
            svg,
            Some(ready.decoded_bytes_permit),
        )
    }

    pub(super) fn complete_shared_ready(
        &mut self,
        identity: &ImageResourceRequestIdentity,
    ) -> bool {
        let Some(resource) = self.ready_by_request.get(&identity.request_key) else {
            return false;
        };
        let Some(slot) = self.slots.get_mut(&identity.element) else {
            return false;
        };
        if slot.identity != *identity {
            return false;
        }
        match slot.state {
            ImageResourceState::Ready(_) => {
                slot.scanned_preload = None;
                true
            }
            ImageResourceState::Pending => {
                slot.state = ImageResourceState::Ready(resource);
                slot.scanned_preload = None;
                true
            }
            ImageResourceState::DecodeQueued(_) | ImageResourceState::Failed => false,
        }
    }

    fn complete_ready(
        &mut self,
        identity: &ImageResourceRequestIdentity,
        descriptor: ImageResponseDescriptor,
        pixels: Option<Arc<moli_image::RgbaImage>>,
        svg: Option<Arc<moli_image::SvgImage>>,
        decoded_bytes_permit: Option<ImageDecodedBytesPermit>,
    ) -> bool {
        let Some(slot) = self.slots.get_mut(&identity.element) else {
            return false;
        };
        if slot.identity != *identity {
            return false;
        }
        let resource = Arc::new(ReadyImageResource {
            descriptor,
            density: identity.request_key.density(),
            pixels,
            svg,
            _decoded_bytes_permit: decoded_bytes_permit,
        });
        self.ready_by_request
            .insert(identity.request_key.clone(), &resource);
        slot.state = ImageResourceState::Ready(resource);
        slot.scanned_preload = None;
        true
    }

    pub(super) fn fail(&mut self, identity: &ImageResourceRequestIdentity) -> bool {
        let Some(slot) = self.slots.get_mut(&identity.element) else {
            return false;
        };
        if slot.identity != *identity {
            return false;
        }
        slot.state = ImageResourceState::Failed;
        slot.scanned_preload = None;
        true
    }

    pub(super) fn ready_for_layout(&self, element: DomHandle) -> Option<ReadyImageForLayout> {
        let slot = self.slots.get(&element)?;
        let ImageResourceState::Ready(resource) = &slot.state else {
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

    pub(super) fn is_ready(&self, element: DomHandle) -> bool {
        self.slots
            .get(&element)
            .is_some_and(|slot| matches!(slot.state, ImageResourceState::Ready(_)))
    }

    pub(super) fn has_ready_request(&self, request_key: &ImageRequestKey) -> bool {
        self.ready_by_request.contains_live(request_key)
    }

    pub(super) fn intrinsic_dimensions(&self, element: DomHandle) -> Option<(f32, f32)> {
        let slot = self.slots.get(&element)?;
        let ImageResourceState::Ready(resource) = &slot.state else {
            return None;
        };
        Some(intrinsic_dimensions(resource))
    }

    pub(super) fn retire_element(&mut self, element: DomHandle) -> bool {
        let removed = self.slots.remove(&element).is_some();
        self.prune_dead_ready_requests();
        removed
    }

    pub(super) fn retire_document(&mut self, document: DomHandle) -> usize {
        let before = self.slots.len();
        self.slots
            .retain(|_, slot| slot.identity.document_handle != document);
        self.prune_dead_ready_requests();
        before - self.slots.len()
    }

    fn prune_dead_ready_requests(&mut self) {
        self.ready_by_request.prune_dead();
    }
}

pub(super) fn intrinsic_dimensions(resource: &ReadyImageResource) -> (f32, f32) {
    let density = if resource.density.is_finite() && resource.density > 0.0 {
        resource.density as f32
    } else {
        1.0
    };
    // `ImageResponseDescriptor::{width,height}` deliberately stores integer
    // dimensions for the HTMLImageElement naturalWidth/naturalHeight surface.
    // SVG layout cannot reuse those rounded values: a viewBox-only 96:12 SVG
    // has a 300x37.5 concrete object size even though the DOM reports 300x38.
    // Preserve the metadata's fractional concrete size until the DOM getter's
    // explicit integer conversion boundary.
    let (width, height) = match resource.descriptor.decode_metadata {
        super::ImageDecodeMetadata::Raster(_) => (
            resource.descriptor.width as f32,
            resource.descriptor.height as f32,
        ),
        super::ImageDecodeMetadata::Svg(metadata) => {
            (metadata.concrete_width, metadata.concrete_height)
        }
    };
    (width / density, height / density)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_layout_dimensions_preserve_fractional_concrete_size() {
        let metadata =
            moli_image::svg_image_metadata_from_root_attributes(None, None, Some("0 0 96 12"));
        let descriptor = ImageResponseDescriptor::svg(metadata).expect("valid SVG descriptor");
        assert_eq!((descriptor.width, descriptor.height), (300, 38));

        let resource = ReadyImageResource {
            descriptor,
            density: 1.0,
            pixels: None,
            svg: None,
            _decoded_bytes_permit: None,
        };
        assert_eq!(intrinsic_dimensions(&resource), (300.0, 37.5));

        let high_density_resource = ReadyImageResource {
            density: 2.0,
            ..resource
        };
        assert_eq!(intrinsic_dimensions(&high_density_resource), (150.0, 18.75));
    }
}

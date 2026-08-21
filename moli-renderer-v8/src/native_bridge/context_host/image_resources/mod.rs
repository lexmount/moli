mod budget;
mod css;
mod decode;
mod preload;
mod state;

use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{RendererPageImageLoadEventKind, RendererPageImageLoadEventTaskId},
    types::ImageRequestKey,
};

pub(crate) use css::{CssImageResourceAdmission, CssImageResourceRequestIdentity};
pub(crate) use preload::{ScannedImagePreloadAdmission, SharedScannedImagePreloadLoad};
pub(super) use state::ImageResourceStore;
pub(crate) use state::ReadyImageForLayout;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageResourceRequestIdentity {
    pub(super) element: DomHandle,
    pub(super) sequence: super::ImageLoadEventId,
    pub(super) document_handle: DomHandle,
    pub(super) document_owner: FrameDocumentTaskOwner,
    pub(super) request_key: ImageRequestKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageResponseDescriptor {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(super) decode_metadata: ImageDecodeMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ImageDecodeMetadata {
    Raster(moli_image::RasterImageMetadata),
    Svg(moli_image::SvgImageMetadata),
}

impl ImageDecodeMetadata {
    pub(super) fn retained_byte_len(self, encoded_len: usize) -> Option<usize> {
        match self {
            Self::Raster(metadata) => Some(metadata.decoded_byte_len()),
            Self::Svg(metadata) => metadata.retained_byte_estimate(encoded_len),
        }
    }
}

impl ImageResponseDescriptor {
    pub(crate) const fn raster(metadata: moli_image::RasterImageMetadata) -> Self {
        Self {
            width: metadata.width,
            height: metadata.height,
            decode_metadata: ImageDecodeMetadata::Raster(metadata),
        }
    }

    pub(crate) fn svg(metadata: moli_image::SvgImageMetadata) -> Option<Self> {
        let (width, height) = metadata.concrete_dimensions()?;
        Some(Self {
            width,
            height,
            decode_metadata: ImageDecodeMetadata::Svg(metadata),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageResponseCompletion {
    Ignored,
    Accepted {
        followup: Option<RendererPageImageLoadEventKind>,
    },
}

impl ImageResponseCompletion {
    pub(crate) const fn accepted(self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    pub(crate) const fn followup(self) -> Option<RendererPageImageLoadEventKind> {
        match self {
            Self::Ignored => None,
            Self::Accepted { followup } => followup,
        }
    }
}

impl super::PendingImageLoadEvent {
    fn document_task_owner(self) -> FrameDocumentTaskOwner {
        match self.owner() {
            super::PendingImageLoadEventOwner::Main(binding) => binding.owner(),
            super::PendingImageLoadEventOwner::Child(binding) => binding.owner(),
        }
    }
}

impl super::JsContextHost {
    pub(in crate::native_bridge::context_host) fn visual_resource_generation(&self) -> u64 {
        self.image_resources.visual_generation()
    }

    #[cfg(test)]
    pub(crate) fn visual_resource_generation_handle_for_test(
        &self,
    ) -> super::visual_resource_generation::VisualResourceGeneration {
        self.image_resources.visual_generation_handle_for_test()
    }

    pub(super) fn begin_pending_image_resource(
        &mut self,
        element: DomHandle,
        pending: super::PendingImageLoadEvent,
    ) {
        let Some(request_key) =
            crate::native_bridge::element::image_selected_request_key(self, element)
        else {
            self.image_resources.retire_element(element);
            return;
        };
        self.image_resources.begin(ImageResourceRequestIdentity {
            element,
            sequence: pending.id(),
            document_handle: pending.owner_document_handle(),
            document_owner: pending.document_task_owner(),
            request_key,
        });
    }

    pub(crate) fn retire_image_resource_for_element(&mut self, element: DomHandle) -> bool {
        self.image_resources.retire_element(element)
    }

    pub(crate) fn retire_image_resources_for_document(&mut self, document: DomHandle) -> usize {
        self.image_resources.retire_document(document)
            + self.image_resources.css.retire_document(document)
            + self.image_resources.scanned.retire_document(document)
    }

    pub(crate) fn admit_scanned_image_preload(
        &mut self,
        request_key: ImageRequestKey,
    ) -> ScannedImagePreloadAdmission {
        let Some(document_owner) = self.current_main_document_task_owner() else {
            return ScannedImagePreloadAdmission::Untracked;
        };
        self.image_resources
            .scanned
            .admit(self.document_handle(), document_owner, request_key)
    }

    pub(crate) fn has_scanned_image_preload_for_element(&self, element: DomHandle) -> bool {
        self.image_resources
            .identity(element)
            .is_some_and(|identity| self.image_resources.scanned.contains(identity))
    }

    pub(crate) fn claim_scanned_image_preload_for_element(
        &mut self,
        element: DomHandle,
    ) -> Option<SharedScannedImagePreloadLoad> {
        self.image_resources.claim_scanned_preload(element)
    }

    pub(crate) fn ready_image_for_layout(&self, element: DomHandle) -> Option<ReadyImageForLayout> {
        self.image_resources.ready_for_layout(element)
    }

    pub(crate) fn ready_css_image_for_layout(
        &self,
        document: DomHandle,
        resolved_url: &str,
    ) -> Option<ReadyImageForLayout> {
        self.image_resources
            .css
            .ready_for_layout(document, resolved_url)
    }

    #[cfg(test)]
    pub(crate) fn css_image_resource_observability_for_test(
        &self,
    ) -> (usize, usize, usize, usize, Vec<String>) {
        self.image_resources.css.observability_for_test()
    }

    #[cfg(test)]
    pub(crate) fn css_image_completion_notify_for_test(
        &self,
    ) -> std::sync::Arc<tokio::sync::Notify> {
        self.image_resources.css.completion_notify_for_test()
    }

    pub(crate) fn admit_stylesheet_css_image(
        &mut self,
        binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
        resolved_url: String,
    ) -> CssImageResourceAdmission {
        let document_handle = match binding.child_handle() {
            None if self.main_document_task_owner_is_current(binding.owner()) => {
                self.document_handle()
            }
            Some(child_handle) => {
                let Some(snapshot) = self.frame_owner_current_child_snapshot(child_handle) else {
                    return CssImageResourceAdmission::Untracked;
                };
                let owner = binding.owner();
                if snapshot.scheduler_lane_id != owner.scheduler_lane_id
                    || snapshot.local_window_id != owner.local_window_id
                    || snapshot.document_id != owner.document_id
                {
                    return CssImageResourceAdmission::Untracked;
                }
                snapshot.document_handle
            }
            None => return CssImageResourceAdmission::Untracked,
        };
        self.image_resources
            .css
            .admit(document_handle, binding.owner(), resolved_url)
    }

    pub(crate) fn stylesheet_css_image_is_current(
        &self,
        document_handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        let owner = if document_handle == self.document_handle() {
            self.current_main_document_task_owner()
        } else {
            self.child_browsing_context_host_for_document_handle(document_handle)
                .and_then(|child_handle| self.frame_owner_current_child_snapshot(child_handle))
                .filter(|snapshot| snapshot.document_handle == document_handle)
                .map(|snapshot| {
                    FrameDocumentTaskOwner::new(
                        snapshot.scheduler_lane_id,
                        snapshot.local_window_id,
                        snapshot.document_id,
                    )
                })
        };
        owner.is_some_and(|owner| {
            self.image_resources
                .css
                .contains_current(document_handle, owner, resolved_url)
        })
    }

    pub(crate) fn complete_stylesheet_css_image_response(
        &mut self,
        identity: &CssImageResourceRequestIdentity,
        descriptor: Option<ImageResponseDescriptor>,
        encoded: &[u8],
    ) -> bool {
        let Some(descriptor) = descriptor else {
            return self.image_resources.css.fail(identity);
        };
        if !self
            .image_resources
            .css
            .mark_decode_queued(identity, descriptor)
        {
            return false;
        }
        let submission = self
            .document_resource_loader_for_owner(identity.document_owner)
            .map(|loader| loader.task_runner())
            .ok_or(decode::ImageDecodeQueueError::JobLimit)
            .and_then(|runner| {
                self.image_resources.decode.submit_css(
                    runner,
                    self.image_resources.css.clone(),
                    identity.clone(),
                    descriptor.decode_metadata,
                    encoded,
                )
            });
        if submission.is_ok() {
            tracing::debug!(
                document = identity.document_handle.index(),
                request_id = identity.request_id,
                encoded_bytes = encoded.len(),
                "queued bounded CSS image resource decode"
            );
            return true;
        }
        tracing::debug!(
            document = identity.document_handle.index(),
            request_id = identity.request_id,
            error = ?submission.expect_err("checked failed CSS image decode submission"),
            "CSS image decode queue rejected the resource"
        );
        self.image_resources.css.fail(identity)
    }

    pub(crate) fn fail_stylesheet_css_image(
        &mut self,
        identity: &CssImageResourceRequestIdentity,
    ) -> bool {
        self.image_resources.css.fail(identity)
    }

    pub(crate) fn image_resource_is_ready(&self, element: DomHandle) -> bool {
        self.image_resources.is_ready(element)
    }

    pub(crate) fn has_ready_image_request(&self, request_key: &ImageRequestKey) -> bool {
        self.image_resources.has_ready_request(request_key)
    }

    pub(crate) fn image_resource_intrinsic_dimensions(
        &self,
        element: DomHandle,
    ) -> Option<(u32, u32)> {
        let (width, height) = self.image_resource_intrinsic_size(element)?;
        Some((
            width.max(0.0).round() as u32,
            height.max(0.0).round() as u32,
        ))
    }

    pub(crate) fn image_resource_intrinsic_size(&self, element: DomHandle) -> Option<(f32, f32)> {
        self.image_resources.intrinsic_dimensions(element)
    }

    pub(crate) fn complete_pending_image_load_local_response_if_matches(
        &mut self,
        element: DomHandle,
        sequence: super::ImageLoadEventId,
        descriptor: Option<ImageResponseDescriptor>,
        encoded: &[u8],
    ) -> ImageResponseCompletion {
        self.complete_pending_image_response_if_matches(
            element,
            sequence,
            None,
            super::PendingImageLoadTerminalSource::Local,
            descriptor,
            encoded,
        )
    }

    pub(crate) fn complete_pending_image_load_network_response_if_matches(
        &mut self,
        element: DomHandle,
        sequence: super::ImageLoadEventId,
        internal_id: u64,
        descriptor: Option<ImageResponseDescriptor>,
        encoded: &[u8],
    ) -> ImageResponseCompletion {
        self.complete_pending_image_response_if_matches(
            element,
            sequence,
            Some(internal_id),
            super::PendingImageLoadTerminalSource::Network,
            descriptor,
            encoded,
        )
    }

    fn complete_pending_image_response_if_matches(
        &mut self,
        element: DomHandle,
        sequence: super::ImageLoadEventId,
        internal_id: Option<u64>,
        source: super::PendingImageLoadTerminalSource,
        descriptor: Option<ImageResponseDescriptor>,
        encoded: &[u8],
    ) -> ImageResponseCompletion {
        let Some(pending) = self.pending_image_load_event(element) else {
            return ImageResponseCompletion::Ignored;
        };
        let expected_state = internal_id.map_or(
            super::PendingImageLoadNetworkState::Unbound,
            super::PendingImageLoadNetworkState::Pending,
        );
        if pending.id() != sequence || pending.network_state != expected_state {
            tracing::debug!(
                image = element.index(),
                sequence = sequence.get(),
                ?internal_id,
                "ignored stale image response before decode admission"
            );
            return ImageResponseCompletion::Ignored;
        }
        let Some(descriptor) = descriptor else {
            if let Some(identity) = self.image_resources.identity(element).cloned() {
                let _ = self.image_resources.fail(&identity);
            }
            let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
                return ImageResponseCompletion::Ignored;
            };
            pending.network_state = super::PendingImageLoadNetworkState::Failed(source);
            return ImageResponseCompletion::Accepted {
                followup: super::image_loads::pending_image_load_terminal_followup(pending),
            };
        };
        let Some(identity) = self.image_resources.identity(element).cloned() else {
            let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
                return ImageResponseCompletion::Ignored;
            };
            pending.network_state = super::PendingImageLoadNetworkState::Failed(source);
            return ImageResponseCompletion::Accepted {
                followup: super::image_loads::pending_image_load_terminal_followup(pending),
            };
        };
        if identity.sequence != sequence
            || identity.document_handle != pending.owner_document_handle()
            || identity.document_owner != pending.document_task_owner()
        {
            return ImageResponseCompletion::Ignored;
        }

        let loader =
            self.document_resource_loader_for_dispatch_scope(pending.target().dispatch_scope());
        let decode_enabled = self.layout_policy().uses_real_layout()
            && loader.as_ref().is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Image)
            });
        if decode_enabled && self.image_resources.complete_shared_ready(&identity) {
            let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
                return ImageResponseCompletion::Ignored;
            };
            pending.network_state = super::PendingImageLoadNetworkState::Ready(source);
            return ImageResponseCompletion::Accepted {
                followup: super::image_loads::pending_image_load_terminal_followup(pending),
            };
        }
        if decode_enabled {
            if !self
                .image_resources
                .mark_decode_queued(&identity, descriptor)
            {
                return ImageResponseCompletion::Ignored;
            }
            let submission = loader
                .map(|loader| loader.task_runner())
                .ok_or(decode::ImageDecodeQueueError::JobLimit)
                .and_then(|runner| {
                    self.image_resources.decode.submit(
                        runner,
                        self.page_image_load_event_sender(),
                        identity.clone(),
                        pending.target(),
                        descriptor.decode_metadata,
                        encoded,
                    )
                });
            if submission.is_ok() {
                let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
                    return ImageResponseCompletion::Ignored;
                };
                pending.network_state = super::PendingImageLoadNetworkState::DecodeQueued(source);
                tracing::debug!(
                    image = element.index(),
                    sequence = sequence.get(),
                    encoded_bytes = encoded.len(),
                    "queued bounded image resource decode"
                );
                return ImageResponseCompletion::Accepted { followup: None };
            }
            tracing::debug!(
                image = element.index(),
                sequence = sequence.get(),
                error = ?submission.expect_err("checked failed image decode submission"),
                "image decode queue rejected the resource"
            );
            let _ = self.image_resources.fail(&identity);
            let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
                return ImageResponseCompletion::Ignored;
            };
            pending.network_state = super::PendingImageLoadNetworkState::Failed(source);
            return ImageResponseCompletion::Accepted {
                followup: super::image_loads::pending_image_load_terminal_followup(pending),
            };
        }

        if !self
            .image_resources
            .complete_metadata(&identity, descriptor)
        {
            return ImageResponseCompletion::Ignored;
        }
        let Some(pending) = self.pending_image_load_events.get_mut(&element) else {
            return ImageResponseCompletion::Ignored;
        };
        pending.network_state = super::PendingImageLoadNetworkState::Ready(source);
        ImageResponseCompletion::Accepted {
            followup: super::image_loads::pending_image_load_terminal_followup(pending),
        }
    }

    pub(crate) fn image_decode_completion_kind(
        &self,
        task_id: RendererPageImageLoadEventTaskId,
        target: super::WindowDocumentTaskTarget,
    ) -> Option<RendererPageImageLoadEventKind> {
        let pending = self.pending_image_load_event_task(task_id)?;
        if !matches!(
            pending.network_state,
            super::PendingImageLoadNetworkState::DecodeQueued(_)
        ) {
            return None;
        }
        let identity = self.image_resources.identity(task_id.element())?;
        if identity.sequence != task_id.sequence() {
            return None;
        }
        self.image_resources
            .decode
            .completion_kind(task_id, identity, target)
    }

    pub(crate) fn discard_image_decode_completion(
        &mut self,
        task_id: RendererPageImageLoadEventTaskId,
    ) -> bool {
        self.image_resources.decode.discard_completion(task_id)
    }

    pub(crate) fn commit_image_decode_completion_if_current(
        &mut self,
        task_id: RendererPageImageLoadEventTaskId,
        target: super::WindowDocumentTaskTarget,
        kind: RendererPageImageLoadEventKind,
    ) -> bool {
        let Some(pending) = self.pending_image_load_event_task(task_id) else {
            return false;
        };
        let super::PendingImageLoadNetworkState::DecodeQueued(source) = pending.network_state
        else {
            return false;
        };
        if pending.target() != target {
            return false;
        }
        let Some(identity) = self.image_resources.identity(task_id.element()).cloned() else {
            return false;
        };
        if identity.sequence != task_id.sequence()
            || identity.document_handle != pending.owner_document_handle()
            || identity.document_owner != pending.document_task_owner()
        {
            return false;
        }
        let Some(completion) = self
            .image_resources
            .decode
            .take_completion(task_id, &identity, target, kind)
        else {
            return false;
        };
        let successful = match completion.result {
            decode::ImageDecodeResult::Ready(ready) => {
                self.image_resources.complete_decode(&identity, ready)
            }
            decode::ImageDecodeResult::Failed(error) => {
                tracing::debug!(
                    image = identity.element.index(),
                    sequence = identity.sequence.get(),
                    %error,
                    "image resource decode failed"
                );
                let _ = self.image_resources.fail(&identity);
                false
            }
        };
        let Some(pending) = self.pending_image_load_events.get_mut(&task_id.element()) else {
            return false;
        };
        if pending.id != task_id.sequence() {
            return false;
        }
        pending.network_state = if successful {
            super::PendingImageLoadNetworkState::Ready(source)
        } else {
            super::PendingImageLoadNetworkState::Failed(source)
        };
        pending.terminal_followup_queued = true;
        true
    }
}

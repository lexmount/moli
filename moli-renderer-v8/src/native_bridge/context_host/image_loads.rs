use super::{
    ImageLoadEventId, ImageLoader, PageScriptHost, PendingImageLoadEvent,
    PendingImageLoadEventOwner, PendingImageLoadNetworkState, PendingImageLoadTerminalSource,
    QueuedImageUpdate,
};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    native_bridge::WindowDocumentTaskTarget,
    page_task_queue::{RendererPageImageLoadEventKind, RendererPageImageLoadEventTaskId},
};

use std::collections::hash_map::Entry;

impl ImageLoader {
    fn owner_document_handle(&self) -> DomHandle {
        match self {
            Self::Queued(update) => update.owner_document_handle,
            Self::Deferred(document) => *document,
            Self::Admitted(pending) => pending.owner_document_handle(),
        }
    }
}

impl PageScriptHost {
    pub(in crate::native_bridge) fn image_loader(&self, handle: DomHandle) -> Option<&ImageLoader> {
        self.image_loaders.get(&handle)
    }

    pub(in crate::native_bridge) fn next_image_load_event_id(&mut self) -> ImageLoadEventId {
        let id = ImageLoadEventId::new(self.next_image_load_event_id);
        self.next_image_load_event_id = self
            .next_image_load_event_id
            .checked_add(1)
            .expect("image load-event id space exhausted");
        id
    }

    pub(in crate::native_bridge) fn register_queued_image_update(
        &mut self,
        handle: DomHandle,
        update: QueuedImageUpdate,
    ) -> bool {
        let Entry::Vacant(entry) = self.image_loaders.entry(handle) else {
            return false;
        };
        entry.insert(ImageLoader::Queued(update));
        true
    }

    pub(in crate::native_bridge) fn defer_image_update(
        &mut self,
        handle: DomHandle,
        owner_document_handle: DomHandle,
    ) {
        self.image_loaders
            .entry(handle)
            .or_insert(ImageLoader::Deferred(owner_document_handle));
    }

    pub(in crate::native_bridge) fn take_queued_image_update(
        &mut self,
        handle: DomHandle,
        id: ImageLoadEventId,
    ) -> Option<QueuedImageUpdate> {
        let Entry::Occupied(entry) = self.image_loaders.entry(handle) else {
            return None;
        };
        let ImageLoader::Queued(update) = *entry.get() else {
            return None;
        };
        if update.id != id {
            return None;
        }
        entry.remove();
        (self.dom_host().owner_document_handle(handle) == Some(update.owner_document_handle))
            .then_some(update)
    }

    pub(crate) fn pending_image_load_event(
        &self,
        handle: DomHandle,
    ) -> Option<PendingImageLoadEvent> {
        match self.image_loader(handle)? {
            ImageLoader::Admitted(pending) => Some(*pending),
            _ => None,
        }
    }

    pub(super) fn pending_image_load_event_mut(
        &mut self,
        handle: DomHandle,
    ) -> Option<&mut PendingImageLoadEvent> {
        match self.image_loaders.get_mut(&handle)? {
            ImageLoader::Admitted(pending) => Some(pending),
            _ => None,
        }
    }

    fn take_pending_image_load_event_if_matches(
        &mut self,
        handle: DomHandle,
        id: ImageLoadEventId,
    ) -> Option<PendingImageLoadEvent> {
        let Entry::Occupied(entry) = self.image_loaders.entry(handle) else {
            return None;
        };
        let ImageLoader::Admitted(pending) = *entry.get() else {
            return None;
        };
        if pending.id() != id {
            return None;
        }
        entry.remove();
        Some(pending)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_image_load_event_for_test(&self, handle: DomHandle) -> bool {
        self.pending_image_load_event(handle).is_some()
    }

    #[cfg(test)]
    pub(crate) fn has_pending_image_network_requests(&self) -> bool {
        self.image_loaders
            .values()
            .any(|loader| matches!(loader, ImageLoader::Admitted(pending) if pending.network_request_id().is_some()))
    }

    pub(crate) fn register_pending_image_load_event(
        &mut self,
        image_handle: DomHandle,
        target: WindowDocumentTaskTarget,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
        delays_load: bool,
    ) -> Option<PendingImageLoadEvent> {
        let node = self.dom_host().node(image_handle)?;
        let element = node
            .as_element()
            .filter(|element| element.is_image_resource_element())?;
        let owner_document_handle = node.owner_document()?;
        let resource_timing_initiator_type = if element.is_svg_element("image") {
            "image"
        } else {
            "img"
        };
        let delays_load = match self.image_loader(image_handle) {
            Some(ImageLoader::Queued(_)) => return None,
            // A resumed image keeps the decision made before deferring.
            Some(ImageLoader::Deferred(document)) => {
                delays_load && *document != owner_document_handle
            }
            Some(ImageLoader::Admitted(previous)) => {
                let previous = *previous;
                let is_main_document = owner_document_handle == self.document_handle();
                if is_main_document
                    == matches!(previous.owner(), PendingImageLoadEventOwner::Main(_))
                    && self.pending_image_load_event_is_current(image_handle, previous)
                {
                    return None;
                }
                self.image_loaders.remove(&image_handle);
                let _ = self.settle_pending_image_load_event(previous, true);
                delays_load
            }
            None => delays_load,
        };

        let owner =
            self.accept_image_load_delay(image_handle, owner_document_handle, delays_load)?;
        let pending = PendingImageLoadEvent::new(
            self.next_image_load_event_id(),
            owner_document_handle,
            target,
            owner,
            request_initiator_type,
            resource_timing_initiator_type,
        );
        self.image_loaders
            .insert(image_handle, ImageLoader::Admitted(pending));
        self.begin_pending_image_resource(image_handle, pending);
        match pending.owner() {
            PendingImageLoadEventOwner::Main(binding) => tracing::debug!(
                image = image_handle.index(),
                sequence = pending.id().get(),
                owner = ?binding.owner(),
                token = ?binding.load_delay_token(),
                "accepted main image request sequence"
            ),
            PendingImageLoadEventOwner::Child(binding) => tracing::debug!(
                image = image_handle.index(),
                sequence = pending.id().get(),
                child_handle = binding.child_handle().index(),
                owner = ?binding.owner(),
                token = ?binding.load_delay_token(),
                "accepted child image request sequence"
            ),
        }
        Some(pending)
    }

    fn accept_image_load_delay(
        &mut self,
        image_handle: DomHandle,
        owner_document_handle: DomHandle,
        delays_load: bool,
    ) -> Option<PendingImageLoadEventOwner> {
        Some(if owner_document_handle == self.document_handle() {
            PendingImageLoadEventOwner::Main(
                self.accept_current_main_image_load_delay(image_handle, delays_load)?,
            )
        } else {
            let child_handle =
                self.child_browsing_context_host_for_document_handle(owner_document_handle)?;
            let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
            if snapshot.document_handle != owner_document_handle {
                return None;
            }
            let owner = FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            );
            PendingImageLoadEventOwner::Child(
                self.frame_owner_store
                    .accept_current_child_image_load_event(
                        child_handle,
                        owner,
                        image_handle,
                        delays_load,
                    )?,
            )
        })
    }

    pub(crate) fn pending_image_load_event_is_current(
        &self,
        image_handle: DomHandle,
        pending: PendingImageLoadEvent,
    ) -> bool {
        if self.dom_host().owner_document_handle(image_handle)
            != Some(pending.owner_document_handle())
        {
            return false;
        }
        match pending.owner() {
            PendingImageLoadEventOwner::Main(binding) => {
                binding.element() == image_handle && self.main_image_load_delay_is_current(binding)
            }
            PendingImageLoadEventOwner::Child(binding) => {
                binding.element() == image_handle
                    && self
                        .frame_owner_store
                        .child_image_load_event_binding_is_current(binding)
            }
        }
    }

    pub(crate) fn bind_pending_image_load_network_request_if_matches(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
        internal_id: u64,
    ) -> bool {
        let Some(pending) = self.pending_image_load_event_mut(image_handle) else {
            return false;
        };
        if pending.id != id || pending.network_state != PendingImageLoadNetworkState::Unbound {
            return false;
        }
        pending.network_state = PendingImageLoadNetworkState::Pending(internal_id);
        tracing::debug!(
            image = image_handle.index(),
            sequence = id.get(),
            internal_id,
            "bound image lifecycle sequence to network request"
        );
        true
    }

    pub(crate) fn complete_pending_image_load_local_resource_if_matches(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
        successful: bool,
    ) -> Option<RendererPageImageLoadEventKind> {
        let pending = self.pending_image_load_event_mut(image_handle)?;
        if pending.id != id || pending.network_state != PendingImageLoadNetworkState::Unbound {
            return None;
        }
        let _ = self.retire_image_resource_for_element(image_handle);
        let pending = self.pending_image_load_event_mut(image_handle)?;
        pending.network_state = if successful {
            PendingImageLoadNetworkState::Ready(PendingImageLoadTerminalSource::Local)
        } else {
            PendingImageLoadNetworkState::Failed(PendingImageLoadTerminalSource::Local)
        };
        tracing::debug!(
            image = image_handle.index(),
            sequence = id.get(),
            successful,
            "completed local or policy image resource"
        );
        pending_image_load_terminal_followup(pending)
    }

    pub(crate) fn complete_pending_image_load_reused_resource_if_matches(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
    ) -> Option<RendererPageImageLoadEventKind> {
        if !self.image_resource_is_ready(image_handle) {
            return None;
        }
        let pending = self.pending_image_load_event_mut(image_handle)?;
        if pending.id != id || pending.network_state != PendingImageLoadNetworkState::Unbound {
            return None;
        }
        pending.network_state =
            PendingImageLoadNetworkState::Ready(PendingImageLoadTerminalSource::Reused);
        tracing::debug!(
            image = image_handle.index(),
            sequence = id.get(),
            "reused active decoded image resource without another fetch or decode"
        );
        pending_image_load_terminal_followup(pending)
    }

    pub(crate) fn complete_pending_image_load_network_request_if_matches(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
        internal_id: u64,
        successful: bool,
    ) -> Option<RendererPageImageLoadEventKind> {
        let pending = self.pending_image_load_event_mut(image_handle)?;
        if pending.id != id
            || pending.network_state != PendingImageLoadNetworkState::Pending(internal_id)
        {
            tracing::debug!(
                image = image_handle.index(),
                sequence = id.get(),
                internal_id,
                successful,
                "ignored stale image network terminal"
            );
            return None;
        }
        let _ = self.retire_image_resource_for_element(image_handle);
        let pending = self.pending_image_load_event_mut(image_handle)?;
        pending.network_state = if successful {
            PendingImageLoadNetworkState::Ready(PendingImageLoadTerminalSource::Network)
        } else {
            PendingImageLoadNetworkState::Failed(PendingImageLoadTerminalSource::Network)
        };
        tracing::debug!(
            image = image_handle.index(),
            sequence = id.get(),
            internal_id,
            successful,
            "applied image network terminal to lifecycle sequence"
        );
        pending_image_load_terminal_followup(pending)
    }

    pub(crate) fn pending_image_load_terminal_followup_if_ready(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
    ) -> Option<RendererPageImageLoadEventKind> {
        let pending = self.pending_image_load_event_mut(image_handle)?;
        if pending.id != id {
            return None;
        }
        pending_image_load_terminal_followup(pending)
    }

    pub(crate) fn current_pending_image_load_event_task(
        &self,
        task_id: RendererPageImageLoadEventTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageImageLoadEventKind)> {
        let pending = self
            .pending_image_load_event(task_id.element())
            .filter(|pending| pending.id() == task_id.sequence())?;
        if !self.pending_image_load_event_is_current(task_id.element(), pending) {
            return None;
        }
        let kind = pending
            .terminal_followup()
            .or_else(|| self.image_decode_completion_kind(task_id, pending.target()))?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, kind))
    }

    pub(crate) fn pending_image_load_event_task(
        &self,
        task_id: RendererPageImageLoadEventTaskId,
    ) -> Option<PendingImageLoadEvent> {
        self.pending_image_load_event(task_id.element())
            .filter(|pending| pending.id() == task_id.sequence())
    }

    pub(crate) fn take_pending_image_load_event_task_for_exact_target(
        &mut self,
        task_id: RendererPageImageLoadEventTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageImageLoadEventKind,
    ) -> Option<PendingImageLoadEvent> {
        let pending = self.pending_image_load_event_task(task_id)?;
        if pending.target() != target || pending.terminal_followup() != Some(kind) {
            return None;
        }
        self.take_pending_image_load_event_if_matches(task_id.element(), task_id.sequence())
    }

    pub(crate) fn discard_stale_pending_image_load_event_task(
        &mut self,
        task_id: RendererPageImageLoadEventTaskId,
    ) -> bool {
        let _ = self.discard_image_decode_completion(task_id);
        let Some(pending) =
            self.take_pending_image_load_event_if_matches(task_id.element(), task_id.sequence())
        else {
            return false;
        };
        self.settle_pending_image_load_event(pending, false)
    }

    /// Apply an image event only inside the exact Window context authorized by
    /// the Page arbiter. The target has already passed currentness admission;
    /// this boundary only resolves that target's V8 context and dispatch scope.
    pub(crate) fn apply_authorized_image_load_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        task_id: RendererPageImageLoadEventTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageImageLoadEventKind,
    ) -> Option<crate::page_task_queue::PageImageLoadEventTargetEffect> {
        let resolved = self.resolve_authorized_window_document_task_context(scope, target)?;
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = super::super::element::apply_authorized_image_load_event_in_context(
            scope, self, host_ptr, task_id, target, kind,
        );
        dispatch_scope.restore(scope, previous_scope);
        effect
    }

    pub(crate) fn settle_pending_image_load_event(
        &mut self,
        pending: PendingImageLoadEvent,
        queue_lifecycle_followup: bool,
    ) -> bool {
        if let Some(internal_id) = pending.network_request_id() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            let aborted = self.abort_subresource_fetch(internal_id);
            tracing::debug!(
                sequence = pending.id().get(),
                internal_id,
                aborted,
                "cancelled image network request with lifecycle sequence"
            );
        }
        self.settle_pending_image_load_owner(pending.owner(), queue_lifecycle_followup)
    }

    fn settle_pending_image_load_owner(
        &mut self,
        owner: PendingImageLoadEventOwner,
        queue_lifecycle_followup: bool,
    ) -> bool {
        match owner {
            PendingImageLoadEventOwner::Main(binding) => {
                let settled = self.settle_main_image_load_delay(binding);
                tracing::debug!(
                    owner = ?binding.owner(),
                    image = binding.element().index(),
                    token = ?binding.load_delay_token(),
                    ?settled,
                    "settled main image load-delay binding"
                );
                settled.settled()
            }
            PendingImageLoadEventOwner::Child(binding) => {
                let settled = self
                    .frame_owner_store
                    .settle_child_image_load_event_binding(binding);
                if !settled.settled() {
                    return false;
                }
                if queue_lifecycle_followup && settled.released() {
                    let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                        binding.child_handle(),
                        binding.owner(),
                    );
                }
                true
            }
        }
    }

    pub(in crate::native_bridge) fn cancel_image_load(&mut self, image_handle: DomHandle) {
        if let Some(ImageLoader::Admitted(pending)) = self.image_loaders.remove(&image_handle) {
            let _ = self.settle_pending_image_load_event(pending, true);
        }
    }

    pub(crate) fn cancel_pending_image_load_event_if_matches(
        &mut self,
        image_handle: DomHandle,
        id: ImageLoadEventId,
    ) -> bool {
        let Some(pending) = self.take_pending_image_load_event_if_matches(image_handle, id) else {
            return false;
        };
        self.settle_pending_image_load_event(pending, true)
    }

    pub(in crate::native_bridge::context_host) fn retire_image_state_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> usize {
        let admitted = self
            .image_loaders
            .extract_if(|_, loader| loader.owner_document_handle() == document_handle)
            .filter_map(|(_, loader)| match loader {
                ImageLoader::Admitted(pending) => Some(pending),
                _ => None,
            })
            .collect::<Vec<_>>();
        for pending in &admitted {
            let _ = self.settle_pending_image_load_event(*pending, false);
        }
        let _ = self.retire_image_resources_for_document(document_handle);
        let _ = self.retire_canvas_resources_for_document(document_handle);
        admitted.len()
    }
}

pub(super) fn pending_image_load_terminal_followup(
    pending: &mut PendingImageLoadEvent,
) -> Option<RendererPageImageLoadEventKind> {
    if pending.terminal_followup_queued {
        return None;
    }
    let followup = match pending.network_state {
        PendingImageLoadNetworkState::Ready(_) => RendererPageImageLoadEventKind::Load,
        PendingImageLoadNetworkState::Failed(_) => RendererPageImageLoadEventKind::Error,
        PendingImageLoadNetworkState::Unbound
        | PendingImageLoadNetworkState::Pending(_)
        | PendingImageLoadNetworkState::DecodeQueued(_) => return None,
    };
    pending.terminal_followup_queued = true;
    Some(followup)
}

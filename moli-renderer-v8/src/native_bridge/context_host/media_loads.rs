use super::{
    JsContextHost, MediaLoadSequenceId, OwnerDispatchScope, PendingMediaLoadDelay,
    PendingMediaLoadNetworkState, PendingMediaLoadSequence, WindowExecutionContextOwner,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::Node,
    frame_owner_model::{FrameDocumentMediaLoadDelayBinding, FrameDocumentTaskOwner},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingMediaLoadTerminalFollowup {
    Ready,
    Failed,
}

impl JsContextHost {
    pub(crate) fn register_pending_media_load_sequence(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media_handle: DomHandle,
    ) -> Option<PendingMediaLoadSequence> {
        let owner_document_handle = self
            .dom_host()
            .node(media_handle)
            .filter(|node| {
                node.as_element().is_some_and(|element| {
                    element.is_html_element("audio") || element.is_html_element("video")
                })
            })
            .and_then(Node::owner_document)?;

        if let Some(previous) = self.take_pending_media_load_sequence(media_handle) {
            let _ = self.settle_pending_media_load_sequence(previous, true);
        }

        let task_target = self.media_element_task_target(scope, media_handle)?;
        let task_document_handle = self
            .resolve_media_element_task_context(scope, task_target)?
            .document_handle;
        let load_delay = if owner_document_handle == self.dom_host().document_handle() {
            self.accept_current_main_media_load_delay(media_handle)
                .map(PendingMediaLoadDelay::Main)
        } else {
            self.child_browsing_context_host_for_document_handle(owner_document_handle)
                .and_then(|child_handle| {
                    let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
                    if snapshot.document_handle != owner_document_handle {
                        return None;
                    }
                    let owner = FrameDocumentTaskOwner::new(
                        snapshot.scheduler_lane_id,
                        snapshot.local_window_id,
                        snapshot.document_id,
                    );
                    self.frame_owner_store
                        .accept_current_child_media_load_delay(child_handle, owner, media_handle)
                })
                .map(PendingMediaLoadDelay::Child)
        };
        let pending = PendingMediaLoadSequence::new(
            self.next_media_load_sequence_id(),
            owner_document_handle,
            task_target,
            task_document_handle,
            load_delay,
        );
        if !self.insert_pending_media_load_sequence(media_handle, pending) {
            let _ = self.settle_pending_media_load_sequence(pending, true);
            return None;
        }
        Some(pending)
    }

    pub(crate) fn pending_media_load_sequence_is_current(
        &self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> bool {
        let Some(pending) = self.pending_media_load_sequence(media_handle) else {
            return false;
        };
        if pending.id() != id
            || self
                .dom_host()
                .node(media_handle)
                .and_then(Node::owner_document)
                != Some(pending.owner_document_handle())
        {
            return false;
        }
        let target = pending.task_target();
        self.window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
    }

    #[cfg(test)]
    pub(crate) fn pending_media_text_track_count(
        &self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> Option<usize> {
        self.pending_media_load_sequences
            .get(&media_handle)
            .filter(|pending| pending.id() == id)
            .map(|pending| pending.pending_text_track_count)
    }

    pub(crate) fn bind_pending_media_load_network_request_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
        internal_id: u64,
    ) -> bool {
        let Some(pending) = self.pending_media_load_sequences.get_mut(&media_handle) else {
            return false;
        };
        if pending.id != id || pending.network_state != PendingMediaLoadNetworkState::Unbound {
            return false;
        }
        pending.network_state = PendingMediaLoadNetworkState::Pending(internal_id);
        tracing::debug!(
            media = media_handle.index(),
            sequence = id.get(),
            internal_id,
            "bound media lifecycle sequence to network request"
        );
        true
    }

    pub(crate) fn complete_pending_media_load_local_resource_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
        successful: bool,
    ) -> Option<PendingMediaLoadTerminalFollowup> {
        let pending = self.pending_media_load_sequences.get_mut(&media_handle)?;
        if pending.id != id || pending.network_state != PendingMediaLoadNetworkState::Unbound {
            return None;
        }
        pending.network_state = if successful {
            PendingMediaLoadNetworkState::Ready
        } else {
            PendingMediaLoadNetworkState::Failed
        };
        tracing::debug!(
            media = media_handle.index(),
            sequence = id.get(),
            successful,
            "completed local media resource for lifecycle sequence"
        );
        pending_media_load_terminal_followup(pending)
    }

    pub(crate) fn complete_pending_media_load_network_request_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
        internal_id: u64,
        successful: bool,
    ) -> Option<PendingMediaLoadTerminalFollowup> {
        let pending = self.pending_media_load_sequences.get_mut(&media_handle)?;
        if pending.id != id
            || pending.network_state != PendingMediaLoadNetworkState::Pending(internal_id)
        {
            tracing::debug!(
                media = media_handle.index(),
                sequence = id.get(),
                internal_id,
                successful,
                "ignored stale media network terminal"
            );
            return None;
        }
        pending.network_state = if successful {
            PendingMediaLoadNetworkState::Ready
        } else {
            PendingMediaLoadNetworkState::Failed
        };
        tracing::debug!(
            media = media_handle.index(),
            sequence = id.get(),
            internal_id,
            successful,
            "applied media network terminal to lifecycle sequence"
        );
        pending_media_load_terminal_followup(pending)
    }

    pub(crate) fn mark_pending_media_loadstart_dispatched_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> Option<PendingMediaLoadTerminalFollowup> {
        let pending = self.pending_media_load_sequences.get_mut(&media_handle)?;
        if pending.id != id {
            return None;
        }
        pending.loadstart_dispatched = true;
        pending_media_load_terminal_followup(pending)
    }

    pub(crate) fn settle_pending_media_load_delay_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> bool {
        let Some(pending) = self
            .pending_media_load_sequences
            .get_mut(&media_handle)
            .filter(|pending| pending.id() == id)
        else {
            return false;
        };
        let previous = *pending;
        let binding = pending.load_delay.take();
        pending.delaying_load_event = false;
        let settled = self.settle_media_load_delay(binding, true, Some(id));
        if previous.delaying_load_event {
            self.resume_document_load_after_media(previous);
        }
        settled
    }

    /// A media resource delays its actual Document's load event independently
    /// of the parser transaction that originally accepted it. Windowless media
    /// may use another Window's task source without delaying that Window.
    pub(in crate::native_bridge::context_host) fn media_delays_document_load_event(
        &self,
        document: DomHandle,
    ) -> bool {
        self.pending_media_load_sequences
            .iter()
            .any(|(media, pending)| {
                pending.delaying_load_event
                    && pending.owner_document_handle == document
                    && pending.task_document_handle == document
                    && self.pending_media_load_sequence_is_current(*media, pending.id())
            })
    }

    fn resume_document_load_after_media(&mut self, pending: PendingMediaLoadSequence) {
        let target = pending.task_target();
        if pending.owner_document_handle != pending.task_document_handle
            || !self
                .window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
        {
            return;
        }
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                let Some(owner) = self.current_main_document_task_owner() else {
                    return;
                };
                if target.owner() == WindowExecutionContextOwner::Frame(owner.local_window_id)
                    && !self.media_delays_document_load_event(pending.owner_document_handle)
                {
                    // The existing lifecycle resident retains its exact load
                    // boundary while blocked; this only makes it runnable.
                    self.frame_owner_store
                        .resume_current_main_document_load_after_resources(owner);
                }
            }
            OwnerDispatchScope::Child(child_handle) => {
                let Some(owner) = self
                    .frame_owner_store
                    .current_child_document_task_owner(child_handle)
                else {
                    return;
                };
                if target.owner() == WindowExecutionContextOwner::Frame(owner.local_window_id) {
                    let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                        child_handle,
                        owner,
                    );
                }
            }
            OwnerDispatchScope::LightweightPopup(_) => {}
        }
    }

    fn settle_media_load_delay(
        &mut self,
        delay: Option<PendingMediaLoadDelay>,
        queue_child_lifecycle_followup: bool,
        sequence: Option<MediaLoadSequenceId>,
    ) -> bool {
        match delay {
            Some(PendingMediaLoadDelay::Main(binding)) => {
                self.settle_main_media_load_delay(binding)
            }
            Some(PendingMediaLoadDelay::Child(binding)) => self
                .settle_pending_child_media_load_delay(
                    binding,
                    queue_child_lifecycle_followup,
                    sequence,
                ),
            None => true,
        }
    }

    /// Opening a parser stream reuses the Document and Window. Drop only the
    /// old parser's delay tokens; resource sequences, the media load-event flag,
    /// and Window tasks continue.
    pub(crate) fn release_media_load_delays_for_document(&mut self, document: DomHandle) -> usize {
        let delays = self
            .pending_media_load_sequences
            .values_mut()
            .filter(|pending| {
                pending.owner_document_handle == document
                    || pending.task_document_handle == document
            })
            .filter_map(|pending| pending.load_delay.take().map(|delay| (delay, pending.id)))
            .collect::<Vec<_>>();
        let count = delays.len();
        for (delay, sequence) in delays {
            self.settle_media_load_delay(Some(delay), false, Some(sequence));
        }
        count
    }

    pub(crate) fn finish_pending_media_load_sequence_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> bool {
        let Some(pending) = self.take_pending_media_load_sequence_if_matches(media_handle, id)
        else {
            return false;
        };
        self.settle_pending_media_load_sequence(pending, true)
    }

    pub(crate) fn cancel_pending_media_load_sequence(&mut self, media_handle: DomHandle) -> bool {
        let Some(pending) = self.take_pending_media_load_sequence(media_handle) else {
            return false;
        };
        self.settle_pending_media_load_sequence(pending, true)
    }

    pub(crate) fn cancel_pending_media_load_sequence_if_matches(
        &mut self,
        media_handle: DomHandle,
        id: MediaLoadSequenceId,
    ) -> bool {
        let Some(pending) = self.take_pending_media_load_sequence_if_matches(media_handle, id)
        else {
            return false;
        };
        self.settle_pending_media_load_sequence(pending, true)
    }

    pub(crate) fn cancel_pending_media_loads_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> usize {
        self.retire_media_playback_for_document(document_handle);
        self.cancel_pending_media_load_sequences_for_document(document_handle)
    }

    pub(crate) fn cancel_pending_media_load_sequences_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> usize {
        let handles = self
            .pending_media_load_sequences
            .iter()
            .filter_map(|(handle, pending)| {
                (pending.owner_document_handle() == document_handle
                    || pending.task_document_handle == document_handle)
                    .then_some(*handle)
            })
            .collect::<Vec<_>>();
        for handle in &handles {
            if let Some(pending) = self.take_pending_media_load_sequence(*handle) {
                let _ = self.settle_pending_media_load_sequence(pending, false);
            }
        }
        let lazy_handles = self
            .lazy_media_load_candidates
            .iter()
            .copied()
            .filter(|handle| {
                self.dom_host().node(*handle).and_then(Node::owner_document)
                    == Some(document_handle)
            })
            .collect::<Vec<_>>();
        let lazy_count = lazy_handles.len();
        for handle in lazy_handles {
            self.lazy_media_load_candidates.remove(&handle);
        }
        if !handles.is_empty() || lazy_count > 0 {
            tracing::debug!(
                document = document_handle.index(),
                sequence_count = handles.len(),
                lazy_candidate_count = lazy_count,
                "retired document media lifecycle state"
            );
        }
        handles.len()
    }

    fn settle_pending_media_load_sequence(
        &mut self,
        pending: PendingMediaLoadSequence,
        queue_child_lifecycle_followup: bool,
    ) -> bool {
        self.retire_pending_media_text_track_gates(pending.id());
        if let Some(internal_id) = pending.network_request_id() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            let aborted = self.abort_subresource_fetch(internal_id);
            tracing::debug!(
                sequence = pending.id().get(),
                internal_id,
                aborted,
                "cancelled media network request with lifecycle sequence"
            );
        }
        let settled = self.settle_media_load_delay(
            pending.load_delay(),
            queue_child_lifecycle_followup,
            Some(pending.id()),
        );
        if queue_child_lifecycle_followup && pending.delaying_load_event {
            self.resume_document_load_after_media(pending);
        }
        settled
    }

    fn settle_pending_child_media_load_delay(
        &mut self,
        binding: FrameDocumentMediaLoadDelayBinding,
        queue_lifecycle_followup: bool,
        sequence: Option<MediaLoadSequenceId>,
    ) -> bool {
        let settled = self
            .frame_owner_store
            .settle_child_media_load_delay(binding);
        tracing::debug!(
            owner = ?binding.owner(),
            child_handle = binding.child_handle().index(),
            media = binding.element().index(),
            sequence = sequence.map(MediaLoadSequenceId::get),
            token = ?binding.load_delay_token(),
            settled,
            "settled child media load-delay binding"
        );
        if settled && queue_lifecycle_followup && binding.load_delay_token().is_some() {
            let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                binding.child_handle(),
                binding.owner(),
            );
        }
        settled
    }
}

fn pending_media_load_terminal_followup(
    pending: &mut PendingMediaLoadSequence,
) -> Option<PendingMediaLoadTerminalFollowup> {
    if !pending.loadstart_dispatched || pending.terminal_followup_queued {
        return None;
    }
    let followup = match pending.network_state {
        PendingMediaLoadNetworkState::Ready => PendingMediaLoadTerminalFollowup::Ready,
        PendingMediaLoadNetworkState::Failed => PendingMediaLoadTerminalFollowup::Failed,
        PendingMediaLoadNetworkState::Unbound | PendingMediaLoadNetworkState::Pending(_) => {
            return None;
        }
    };
    pending.terminal_followup_queued = true;
    Some(followup)
}

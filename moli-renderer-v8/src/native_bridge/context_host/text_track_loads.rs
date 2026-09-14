use super::{
    JsContextHost, MediaLoadSequenceId, PendingMediaTextTrackGate,
    PendingTextTrackLoadNetworkState, PendingTextTrackLoadSequence, TextTrackLoadSequenceId,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::Node,
    page_task_queue::{RendererPageTextTrackLoadTaskId, RendererPageTextTrackLoadTaskKind},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingTextTrackLoadTerminalFollowup {
    Ready,
    FetchFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingTextTrackLoadTerminal {
    Ready(String),
    FetchFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingMediaCanPlayFollowup {
    media_handle: DomHandle,
    media_sequence: MediaLoadSequenceId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingMediaTextTrackGateRegistration {
    displaced_canplay_followup: Option<PendingMediaCanPlayFollowup>,
}

pub(crate) struct DiscardedPendingTextTrackLoad {
    media_canplay_followup: Option<PendingMediaCanPlayFollowup>,
}

impl DiscardedPendingTextTrackLoad {
    pub(crate) fn into_media_canplay_followup(self) -> Option<PendingMediaCanPlayFollowup> {
        self.media_canplay_followup
    }
}

impl PendingMediaTextTrackGateRegistration {
    pub(crate) fn displaced_canplay_followup(self) -> Option<PendingMediaCanPlayFollowup> {
        self.displaced_canplay_followup
    }
}

impl PendingMediaCanPlayFollowup {
    pub(crate) fn media_handle(self) -> DomHandle {
        self.media_handle
    }

    pub(crate) fn media_sequence(self) -> MediaLoadSequenceId {
        self.media_sequence
    }
}

impl JsContextHost {
    pub(crate) fn current_pending_text_track_load_task(
        &self,
        task_id: RendererPageTextTrackLoadTaskId,
    ) -> Option<(
        super::WindowDocumentTaskTarget,
        RendererPageTextTrackLoadTaskKind,
    )> {
        let pending = self
            .pending_text_track_load_sequences
            .get(&task_id.track())
            .filter(|pending| pending.id() == task_id.sequence())?;
        if !self.pending_text_track_load_sequence_is_current(task_id.track(), task_id.sequence()) {
            return None;
        }
        let kind = match (&pending.network_state, pending.terminal_followup_queued) {
            (PendingTextTrackLoadNetworkState::Unbound, false) => {
                RendererPageTextTrackLoadTaskKind::Start
            }
            (PendingTextTrackLoadNetworkState::Ready(_), true) => {
                RendererPageTextTrackLoadTaskKind::NetworkTerminal
            }
            (PendingTextTrackLoadNetworkState::FetchFailed, true) => {
                RendererPageTextTrackLoadTaskKind::FetchFailureTerminal
            }
            (PendingTextTrackLoadNetworkState::Pending(_), _)
            | (PendingTextTrackLoadNetworkState::Unbound, true)
            | (PendingTextTrackLoadNetworkState::Ready(_), false)
            | (PendingTextTrackLoadNetworkState::FetchFailed, false) => return None,
        };
        Some((pending.target(), kind))
    }

    pub(crate) fn send_text_track_load_task(
        &self,
        task_id: RendererPageTextTrackLoadTaskId,
        kind: RendererPageTextTrackLoadTaskKind,
    ) -> Result<(), crate::page_task_queue::RendererPageTextTrackLoadRouteClosed> {
        let pending = self
            .pending_text_track_load_sequences
            .get(&task_id.track())
            .filter(|pending| pending.id() == task_id.sequence())
            .ok_or(crate::page_task_queue::RendererPageTextTrackLoadRouteClosed)?;
        self.page_text_track_load_sender()
            .send(pending.target(), task_id, kind)
    }

    pub(crate) fn apply_authorized_text_track_load_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task_id: RendererPageTextTrackLoadTaskId,
        target: super::WindowDocumentTaskTarget,
        kind: RendererPageTextTrackLoadTaskKind,
    ) -> Option<bool> {
        if self.current_pending_text_track_load_task(task_id) != Some((target, kind)) {
            return None;
        }
        let resolved = self.resolve_authorized_window_document_task_context(scope, target)?;
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let applied =
            super::super::element::apply_text_track_load_task(scope, host_ptr, task_id, kind);
        dispatch_scope.restore(scope, previous_scope);
        Some(applied)
    }

    pub(crate) fn discard_pending_text_track_load_task(
        &mut self,
        task_id: RendererPageTextTrackLoadTaskId,
    ) -> Option<DiscardedPendingTextTrackLoad> {
        if self
            .pending_text_track_load_sequences
            .get(&task_id.track())
            .is_none_or(|pending| pending.id() != task_id.sequence())
        {
            return None;
        }
        Some(DiscardedPendingTextTrackLoad {
            media_canplay_followup: self
                .cancel_pending_text_track_load_sequence(task_id.track(), true),
        })
    }

    pub(crate) fn register_pending_text_track_load_sequence(
        &mut self,
        track_handle: DomHandle,
        source: String,
    ) -> Option<PendingTextTrackLoadSequence> {
        let media_handle = self
            .dom_host()
            .node(track_handle)
            .and_then(Node::parent_node)
            .filter(|parent| {
                self.dom_host().is_html_element_named(*parent, "audio")
                    || self.dom_host().is_html_element_named(*parent, "video")
            })?;
        let owner_document_handle = self.dom_host().owner_document_handle(track_handle)?;
        if self.dom_host().owner_document_handle(media_handle) != Some(owner_document_handle) {
            return None;
        }

        if let Some(previous) = self.pending_text_track_load_sequences.remove(&track_handle) {
            self.abort_pending_text_track_request(&previous);
        }

        let dispatch_scope = if owner_document_handle == self.document_handle() {
            super::OwnerDispatchScope::Top
        } else {
            self.child_browsing_context_host_for_document_handle(owner_document_handle)
                .and_then(|child_handle| {
                    let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
                    (snapshot.document_handle == owner_document_handle)
                        .then_some(super::OwnerDispatchScope::Child(child_handle))
                })?
        };
        let target = self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)?;
        let pending = PendingTextTrackLoadSequence::new(
            self.next_text_track_load_sequence_id(),
            owner_document_handle,
            media_handle,
            target,
            source,
        );
        self.pending_text_track_load_sequences
            .insert(track_handle, pending.clone());
        tracing::debug!(
            track = track_handle.index(),
            media = media_handle.index(),
            sequence = pending.id().get(),
            ?target,
            source = pending.source(),
            "accepted owner-bound text-track request sequence"
        );
        Some(pending)
    }

    pub(crate) fn pending_text_track_load_sequence_is_current(
        &self,
        track_handle: DomHandle,
        sequence: TextTrackLoadSequenceId,
    ) -> bool {
        let Some(pending) = self
            .pending_text_track_load_sequences
            .get(&track_handle)
            .filter(|pending| pending.id() == sequence)
        else {
            return false;
        };
        if self.dom_host().owner_document_handle(track_handle)
            != Some(pending.owner_document_handle())
            || self.dom_host().parent_node(track_handle) != Some(pending.media_handle())
        {
            return false;
        }
        self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        ) == Some(pending.target())
    }

    pub(crate) fn bind_pending_text_track_network_request_if_matches(
        &mut self,
        track_handle: DomHandle,
        sequence: TextTrackLoadSequenceId,
        internal_id: u64,
    ) -> bool {
        let Some(pending) = self
            .pending_text_track_load_sequences
            .get_mut(&track_handle)
        else {
            return false;
        };
        if pending.id != sequence
            || pending.network_state != PendingTextTrackLoadNetworkState::Unbound
        {
            return false;
        }
        pending.network_state = PendingTextTrackLoadNetworkState::Pending(internal_id);
        tracing::debug!(
            track = track_handle.index(),
            sequence = sequence.get(),
            internal_id,
            "bound text-track sequence to network request"
        );
        true
    }

    pub(crate) fn complete_pending_text_track_local_if_matches(
        &mut self,
        track_handle: DomHandle,
        sequence: TextTrackLoadSequenceId,
        result: Result<String, String>,
    ) -> Option<PendingTextTrackLoadTerminalFollowup> {
        let pending = self
            .pending_text_track_load_sequences
            .get_mut(&track_handle)?;
        if pending.id != sequence
            || pending.network_state != PendingTextTrackLoadNetworkState::Unbound
        {
            return None;
        }
        pending.network_state = match result {
            Ok(body) => PendingTextTrackLoadNetworkState::Ready(body),
            Err(_) => PendingTextTrackLoadNetworkState::FetchFailed,
        };
        pending_text_track_terminal_followup(pending)
    }

    pub(crate) fn complete_pending_text_track_network_if_matches(
        &mut self,
        track_handle: DomHandle,
        sequence: TextTrackLoadSequenceId,
        internal_id: u64,
        result: Result<String, String>,
    ) -> Option<PendingTextTrackLoadTerminalFollowup> {
        let pending = self
            .pending_text_track_load_sequences
            .get_mut(&track_handle)?;
        if pending.id != sequence
            || pending.network_state != PendingTextTrackLoadNetworkState::Pending(internal_id)
        {
            tracing::debug!(
                track = track_handle.index(),
                sequence = sequence.get(),
                internal_id,
                "ignored stale text-track network terminal"
            );
            return None;
        }
        pending.network_state = match result {
            Ok(body) => PendingTextTrackLoadNetworkState::Ready(body),
            Err(_) => PendingTextTrackLoadNetworkState::FetchFailed,
        };
        tracing::debug!(
            track = track_handle.index(),
            sequence = sequence.get(),
            internal_id,
            successful = matches!(
                pending.network_state,
                PendingTextTrackLoadNetworkState::Ready(_)
            ),
            "applied text-track network terminal"
        );
        pending_text_track_terminal_followup(pending)
    }

    pub(crate) fn take_pending_text_track_terminal_if_matches(
        &mut self,
        track_handle: DomHandle,
        sequence: TextTrackLoadSequenceId,
    ) -> Option<PendingTextTrackLoadTerminal> {
        let pending =
            self.take_pending_text_track_load_sequence_if_matches(track_handle, sequence)?;
        match pending.network_state {
            PendingTextTrackLoadNetworkState::Ready(body) => {
                Some(PendingTextTrackLoadTerminal::Ready(body))
            }
            PendingTextTrackLoadNetworkState::FetchFailed => {
                Some(PendingTextTrackLoadTerminal::FetchFailed)
            }
            PendingTextTrackLoadNetworkState::Unbound
            | PendingTextTrackLoadNetworkState::Pending(_) => {
                self.pending_text_track_load_sequences
                    .insert(track_handle, pending);
                None
            }
        }
    }

    pub(crate) fn cancel_pending_text_track_load_sequence(
        &mut self,
        track_handle: DomHandle,
        settle_media_gate: bool,
    ) -> Option<PendingMediaCanPlayFollowup> {
        if let Some(pending) = self.pending_text_track_load_sequences.remove(&track_handle) {
            self.abort_pending_text_track_request(&pending);
        }
        settle_media_gate
            .then(|| self.settle_pending_media_text_track_gate(track_handle))
            .flatten()
    }

    pub(in crate::native_bridge::context_host) fn cancel_pending_text_track_loads_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> usize {
        let handles = self
            .pending_text_track_load_sequences
            .iter()
            .filter_map(|(handle, pending)| {
                (pending.owner_document_handle() == document_handle).then_some(*handle)
            })
            .collect::<Vec<_>>();
        for handle in &handles {
            if let Some(pending) = self.pending_text_track_load_sequences.remove(handle) {
                self.abort_pending_text_track_request(&pending);
            }
            self.pending_media_text_track_gates.remove(handle);
        }
        handles.len()
    }

    pub(crate) fn register_pending_media_text_track_gate(
        &mut self,
        media_handle: DomHandle,
        media_sequence: MediaLoadSequenceId,
        track_handle: DomHandle,
    ) -> Option<PendingMediaTextTrackGateRegistration> {
        if !self
            .pending_media_load_sequences
            .get(&media_handle)
            .is_some_and(|pending| pending.id() == media_sequence)
        {
            return None;
        }
        if self
            .pending_media_text_track_gates
            .get(&track_handle)
            .is_some_and(|gate| {
                gate.media_handle == media_handle && gate.media_sequence == media_sequence
            })
        {
            return Some(PendingMediaTextTrackGateRegistration {
                displaced_canplay_followup: None,
            });
        }
        let displaced_canplay_followup = self
            .pending_media_text_track_gates
            .remove(&track_handle)
            .and_then(|previous| self.decrement_pending_media_text_track_count(previous));
        let media = self.pending_media_load_sequences.get_mut(&media_handle)?;
        if media.id() != media_sequence {
            return None;
        }
        media.pending_text_track_count = media.pending_text_track_count.saturating_add(1);
        self.pending_media_text_track_gates.insert(
            track_handle,
            PendingMediaTextTrackGate {
                media_handle,
                media_sequence,
            },
        );
        tracing::debug!(
            media = media_handle.index(),
            media_sequence = media_sequence.get(),
            track = track_handle.index(),
            pending_tracks = media.pending_text_track_count,
            "registered media-selection text-track readiness gate"
        );
        Some(PendingMediaTextTrackGateRegistration {
            displaced_canplay_followup,
        })
    }

    pub(crate) fn defer_pending_media_canplay_for_text_tracks(
        &mut self,
        media_handle: DomHandle,
        media_sequence: MediaLoadSequenceId,
    ) -> bool {
        let Some(media) = self
            .pending_media_load_sequences
            .get_mut(&media_handle)
            .filter(|pending| pending.id() == media_sequence)
        else {
            return false;
        };
        if media.pending_text_track_count == 0 {
            return false;
        }
        media.canplay_waiting_for_text_tracks = true;
        tracing::debug!(
            media = media_handle.index(),
            media_sequence = media_sequence.get(),
            pending_tracks = media.pending_text_track_count,
            "deferred media canplay until selection-time text tracks finish"
        );
        true
    }

    pub(crate) fn settle_pending_media_text_track_gate(
        &mut self,
        track_handle: DomHandle,
    ) -> Option<PendingMediaCanPlayFollowup> {
        let gate = self.pending_media_text_track_gates.remove(&track_handle)?;
        self.decrement_pending_media_text_track_count(gate)
    }

    pub(crate) fn retire_pending_media_text_track_gates(
        &mut self,
        media_sequence: MediaLoadSequenceId,
    ) {
        self.pending_media_text_track_gates
            .retain(|_, gate| gate.media_sequence != media_sequence);
    }

    fn decrement_pending_media_text_track_count(
        &mut self,
        gate: PendingMediaTextTrackGate,
    ) -> Option<PendingMediaCanPlayFollowup> {
        let media = self
            .pending_media_load_sequences
            .get_mut(&gate.media_handle)
            .filter(|pending| pending.id() == gate.media_sequence)?;
        media.pending_text_track_count = media.pending_text_track_count.saturating_sub(1);
        tracing::debug!(
            media = gate.media_handle.index(),
            media_sequence = gate.media_sequence.get(),
            pending_tracks = media.pending_text_track_count,
            "settled media-selection text-track readiness gate"
        );
        if media.pending_text_track_count == 0 && media.canplay_waiting_for_text_tracks {
            media.canplay_waiting_for_text_tracks = false;
            return Some(PendingMediaCanPlayFollowup {
                media_handle: gate.media_handle,
                media_sequence: gate.media_sequence,
            });
        }
        None
    }

    fn abort_pending_text_track_request(&mut self, pending: &PendingTextTrackLoadSequence) {
        let Some(internal_id) = pending.network_request_id() else {
            return;
        };
        let aborted = self.abort_subresource_fetch(internal_id);
        tracing::debug!(
            track_sequence = pending.id().get(),
            internal_id,
            aborted,
            "cancelled exact text-track network request"
        );
    }
}

fn pending_text_track_terminal_followup(
    pending: &mut PendingTextTrackLoadSequence,
) -> Option<PendingTextTrackLoadTerminalFollowup> {
    if pending.terminal_followup_queued {
        return None;
    }
    let followup = match pending.network_state {
        PendingTextTrackLoadNetworkState::Ready(_) => PendingTextTrackLoadTerminalFollowup::Ready,
        PendingTextTrackLoadNetworkState::FetchFailed => {
            PendingTextTrackLoadTerminalFollowup::FetchFailed
        }
        PendingTextTrackLoadNetworkState::Unbound
        | PendingTextTrackLoadNetworkState::Pending(_) => return None,
    };
    pending.terminal_followup_queued = true;
    Some(followup)
}

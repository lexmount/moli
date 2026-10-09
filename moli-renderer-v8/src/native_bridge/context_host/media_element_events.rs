use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, MediaLoadSequenceId, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    native_bridge::element::MediaLoadEventPhase,
    page_task_queue::{RendererPageMediaElementEventTaskId, RendererPageMediaElementEventTaskKind},
};

pub(super) enum PendingMediaElementEventPayload {
    Seeking {
        media_handle: DomHandle,
    },
    SeekCompletion {
        media_handle: DomHandle,
        seek_token: u64,
    },
    LoadEventPhase {
        media_handle: DomHandle,
        sequence: MediaLoadSequenceId,
        phase: MediaLoadEventPhase,
    },
    TextTrackListEvent {
        list: v8::Global<v8::Object>,
        track: v8::Global<v8::Object>,
        event_type: String,
    },
    RemotePlaybackAvailability(
        crate::context_bootstrap::remote_playback::RemotePlaybackAvailabilityTask,
    ),
}

pub(super) type MediaElementEventState = ExactWindowDocumentTaskLedger<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
    PendingMediaElementEventPayload,
>;

impl JsContextHost {
    pub(crate) fn queue_remote_playback_availability(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        callback: crate::context_bootstrap::remote_playback::RemotePlaybackAvailabilityTask,
    ) -> bool {
        self.queue_media_element_event_for_node(
            scope,
            media,
            RendererPageMediaElementEventTaskKind::RemotePlaybackAvailability,
            PendingMediaElementEventPayload::RemotePlaybackAvailability(callback),
        )
    }

    pub(crate) fn queue_media_seeking_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media_handle: DomHandle,
    ) -> bool {
        self.queue_media_element_event_for_node(
            scope,
            media_handle,
            RendererPageMediaElementEventTaskKind::Seeking,
            PendingMediaElementEventPayload::Seeking { media_handle },
        )
    }

    pub(crate) fn queue_media_seek_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media_handle: DomHandle,
        seek_token: u64,
    ) -> bool {
        self.queue_media_element_event_for_node(
            scope,
            media_handle,
            RendererPageMediaElementEventTaskKind::SeekCompletion,
            PendingMediaElementEventPayload::SeekCompletion {
                media_handle,
                seek_token,
            },
        )
    }

    pub(crate) fn queue_media_load_event_phase(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media_handle: DomHandle,
        sequence: MediaLoadSequenceId,
        phase: MediaLoadEventPhase,
    ) -> bool {
        self.queue_media_element_event_for_node(
            scope,
            media_handle,
            RendererPageMediaElementEventTaskKind::LoadEventPhase,
            PendingMediaElementEventPayload::LoadEventPhase {
                media_handle,
                sequence,
                phase,
            },
        )
    }

    pub(crate) fn queue_text_track_list_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media_handle: DomHandle,
        list: v8::Local<'_, v8::Object>,
        track: v8::Local<'_, v8::Object>,
        event_type: String,
    ) -> bool {
        self.queue_media_element_event_for_node(
            scope,
            media_handle,
            RendererPageMediaElementEventTaskKind::TextTrackListEvent,
            PendingMediaElementEventPayload::TextTrackListEvent {
                list: v8::Global::new(scope, list),
                track: v8::Global::new(scope, track),
                event_type,
            },
        )
    }

    fn queue_media_element_event_for_node(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        node: DomHandle,
        kind: RendererPageMediaElementEventTaskKind,
        payload: PendingMediaElementEventPayload,
    ) -> bool {
        let Some(target) = self.window_document_task_target_for_node(scope, node) else {
            self.retire_media_element_event_payload(payload);
            return false;
        };
        let task_id = self
            .media_element_events
            .allocate_task_id(RendererPageMediaElementEventTaskId::from_raw);
        self.media_element_events
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        if self
            .page_media_element_event_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        if let Some(pending) = self
            .media_element_events
            .remove_exact(task_id, target, kind)
        {
            self.retire_media_element_event_payload(pending.into_payload());
        }
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired media-element event after stable route closure"
        );
        false
    }

    pub(crate) fn current_pending_media_element_event_task(
        &self,
        task_id: RendererPageMediaElementEventTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageMediaElementEventTaskKind,
    )> {
        let pending = self.media_element_events.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    /// Consume one already-authorized event before dispatch. Reentrant media
    /// work therefore receives a fresh task id and a later source turn.
    pub(crate) fn apply_authorized_media_element_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task_id: RendererPageMediaElementEventTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageMediaElementEventTaskKind,
    ) -> Option<bool> {
        let payload = self
            .media_element_events
            .remove_exact(task_id, target, kind)?
            .into_payload();
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            self.retire_media_element_event_payload(payload);
            return Some(false);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let dispatched = match payload {
            PendingMediaElementEventPayload::RemotePlaybackAvailability(callback) => {
                callback.invoke(scope, host_ptr)
            }
            PendingMediaElementEventPayload::Seeking { media_handle } => {
                crate::native_bridge::element::dispatch_media_seeking_event(
                    scope,
                    host_ptr,
                    media_handle,
                )
            }
            PendingMediaElementEventPayload::SeekCompletion {
                media_handle,
                seek_token,
            } => crate::native_bridge::element::dispatch_media_seek_completion(
                scope,
                host_ptr,
                media_handle,
                seek_token,
            ),
            PendingMediaElementEventPayload::LoadEventPhase {
                media_handle,
                sequence,
                phase,
            } => crate::native_bridge::element::dispatch_media_load_event_phase(
                scope,
                host_ptr,
                media_handle,
                sequence,
                phase,
            ),
            PendingMediaElementEventPayload::TextTrackListEvent {
                list,
                track,
                event_type,
            } => {
                let list = v8::Local::new(scope, &list);
                let track = v8::Local::new(scope, &track);
                crate::native_bridge::element::dispatch_text_track_list_event(
                    scope,
                    list,
                    track,
                    &event_type,
                )
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        Some(dispatched)
    }

    pub(crate) fn discard_pending_media_element_event_task(
        &mut self,
        task_id: RendererPageMediaElementEventTaskId,
    ) -> bool {
        let Some(pending) = self.media_element_events.remove(task_id) else {
            return false;
        };
        self.retire_media_element_event_payload(pending.into_payload());
        true
    }

    fn retire_media_element_event_payload(&mut self, payload: PendingMediaElementEventPayload) {
        match payload {
            PendingMediaElementEventPayload::SeekCompletion {
                media_handle,
                seek_token,
            } if self.dom_host().media_seek_token(media_handle) == Some(seek_token) => {
                let _ = self.set_media_seeking(media_handle, false);
            }
            PendingMediaElementEventPayload::LoadEventPhase {
                media_handle,
                sequence,
                ..
            } => {
                let _ = self.cancel_pending_media_load_sequence_if_matches(media_handle, sequence);
            }
            PendingMediaElementEventPayload::Seeking { .. }
            | PendingMediaElementEventPayload::RemotePlaybackAvailability(_)
            | PendingMediaElementEventPayload::SeekCompletion { .. }
            | PendingMediaElementEventPayload::TextTrackListEvent { .. } => {}
        }
    }
}

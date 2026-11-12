use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{PageScriptHost, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    page_task_queue::{
        RendererPageTextTrackDefaultModeTaskId, RendererPageTextTrackDefaultModeTaskKind,
    },
};

pub(super) type TextTrackDefaultModeState = ExactWindowDocumentTaskLedger<
    RendererPageTextTrackDefaultModeTaskId,
    RendererPageTextTrackDefaultModeTaskKind,
    DomHandle,
>;

impl PageScriptHost {
    /// Queue one automatic default-mode application on the shared
    /// DOM-manipulation source. Repeated admission for the same exact track is
    /// coalesced until application; a closed Page route never falls back to a
    /// zero-delay timer.
    pub(crate) fn queue_text_track_default_mode_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        track: DomHandle,
    ) -> bool {
        let Some(target) = self.window_document_task_target_for_node(scope, track) else {
            return false;
        };
        let kind = RendererPageTextTrackDefaultModeTaskKind::Apply;
        if self
            .text_track_default_modes
            .find_slot_index(target, kind, |pending| *pending == track)
            .is_some()
        {
            return true;
        }

        let task_id = self
            .text_track_default_modes
            .allocate_task_id(RendererPageTextTrackDefaultModeTaskId::from_raw);
        self.text_track_default_modes
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, track,
            ));
        if self
            .page_text_track_default_mode_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        let removed = self
            .text_track_default_modes
            .remove_exact(task_id, target, kind);
        debug_assert_eq!(
            removed.as_ref().map(|pending| *pending.payload()),
            Some(track)
        );
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            track = track.index(),
            "retired text-track default-mode task after DOM-manipulation route closure"
        );
        false
    }

    pub(crate) fn current_pending_text_track_default_mode_task(
        &self,
        task_id: RendererPageTextTrackDefaultModeTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageTextTrackDefaultModeTaskKind,
    )> {
        let pending = self.text_track_default_modes.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    /// Consume and apply one task already authorized against its exact
    /// Window/Document. Removal precedes application so reentrant attribute or
    /// tree changes can enqueue a distinct tail task.
    pub(crate) fn apply_authorized_text_track_default_mode(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        task_id: RendererPageTextTrackDefaultModeTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageTextTrackDefaultModeTaskKind,
    ) -> Option<bool> {
        let track = self
            .text_track_default_modes
            .remove_exact(task_id, target, kind)?
            .into_payload();
        if self.window_document_task_target_for_node(scope, track) != Some(target) {
            return Some(false);
        }
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return Some(false);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let applied =
            super::super::element::apply_default_text_track_mode_for_track(scope, host_ptr, track);
        dispatch_scope.restore(scope, previous_scope);
        Some(applied)
    }

    pub(crate) fn discard_pending_text_track_default_mode_task(
        &mut self,
        task_id: RendererPageTextTrackDefaultModeTaskId,
    ) -> bool {
        self.text_track_default_modes.remove(task_id).is_some()
    }
}

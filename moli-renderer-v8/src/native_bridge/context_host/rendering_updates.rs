use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::{
    document_runtime::EventTargetHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingRenderingUpdatePayload {
    DocumentScrollEvents,
    AnimationStartScan(EventTargetHandle),
    /// Flush the main Document's autofocus candidates after DOMContentLoaded.
    /// The candidate is intentionally resolved at execution time.
    PostParseAutofocus,
    /// Run the post-font-load layout barrier for one exact loading cycle, then
    /// settle that cycle's `FontFaceSet.ready` projection.
    FontFaceSetReady(crate::script_vm::web_fonts::DocumentWebFontLoadCycleId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostParseAutofocusAdmission {
    /// The exact lifecycle owner was replaced before admission.
    StaleOwner,
    /// Focus already exists or the Document has no eligible candidate.
    NotNeeded,
    /// One exact rendering-update entry owns the pending flush.
    Published,
    /// The Page rendering route retired before the entry could be stored.
    RouteClosed,
}

pub(super) type RenderingUpdateState = ExactWindowDocumentTaskLedger<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
    PendingRenderingUpdatePayload,
>;

impl JsContextHost {
    pub(crate) fn queue_document_web_font_ready_rendering_update_if_needed(&mut self) -> bool {
        let Some(cycle) = self.document_web_font_ready_layout_task_needed() else {
            return false;
        };
        let Some(owner) = self.current_main_document_task_owner() else {
            return false;
        };
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(owner),
            OwnerDispatchScope::Top,
        );
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::FontFaceSetReady,
            PendingRenderingUpdatePayload::FontFaceSetReady(cycle),
        )
    }

    /// Publish post-parse autofocus as a rendering update for the exact main
    /// Document that just completed DOMContentLoaded.
    ///
    /// DOMContentLoaded listeners and their microtasks run before this
    /// admission. Re-checking the exact owner here prevents a listener's
    /// `document.open()` from retargeting old lifecycle work to the replacement
    /// Document.
    pub(crate) fn queue_main_document_post_parse_autofocus(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> PostParseAutofocusAdmission {
        if !self.main_document_task_owner_is_current(owner) {
            return PostParseAutofocusAdmission::StaleOwner;
        }
        if !super::super::element::post_parse_autofocus_is_pending(self) {
            return PostParseAutofocusAdmission::NotNeeded;
        }
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(owner),
            OwnerDispatchScope::Top,
        );
        if self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus,
            PendingRenderingUpdatePayload::PostParseAutofocus,
        ) {
            PostParseAutofocusAdmission::Published
        } else {
            PostParseAutofocusAdmission::RouteClosed
        }
    }

    /// Queue the Document's pending `scroll` and `scrollend` entries into the
    /// stable rendering source. Route closure retires the Host-local payload;
    /// it never falls back to a timer or hidden drain.
    pub(crate) fn queue_document_scroll_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            PendingRenderingUpdatePayload::DocumentScrollEvents,
        )
    }

    /// Queue one lightweight CSS `animationstart` compatibility scan on the
    /// owning Document's rendering source. Repeated listener registration for
    /// the same event target coalesces until that scan is consumed.
    pub(crate) fn queue_animation_start_scan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        event_target: EventTargetHandle,
    ) -> bool {
        let target = match event_target {
            EventTargetHandle::Window => self.current_window_document_task_target(scope),
            EventTargetHandle::Node(handle) => {
                self.window_document_task_target_for_node(scope, handle)
            }
            EventTargetHandle::ChildWindow(_) => None,
        };
        let Some(target) = target else {
            return false;
        };
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan,
            PendingRenderingUpdatePayload::AnimationStartScan(event_target),
        )
    }

    fn queue_rendering_update(
        &mut self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
        payload: PendingRenderingUpdatePayload,
    ) -> bool {
        if self
            .rendering_updates
            .find_slot_index(target, kind, |pending| *pending == payload)
            .is_some()
        {
            return true;
        }

        let task_id = self
            .rendering_updates
            .allocate_task_id(RendererPageRenderingUpdateTaskId::from_raw);
        self.rendering_updates
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        if self
            .page_rendering_update_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        let removed = self.rendering_updates.remove_exact(task_id, target, kind);
        debug_assert_eq!(
            removed.as_ref().map(|pending| *pending.payload()),
            Some(payload)
        );
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired rendering update after stable route closure"
        );
        false
    }

    pub(crate) fn current_pending_rendering_update_task(
        &self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageRenderingUpdateTaskKind,
    )> {
        let pending = self.rendering_updates.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    /// Consume and apply one update already authorized against its exact
    /// Window/Document owner. Removal happens before dispatch so reentrant work
    /// always receives a distinct subsequent rendering turn.
    pub(crate) fn apply_authorized_rendering_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task_id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> Option<bool> {
        let payload = self
            .rendering_updates
            .remove_exact(task_id, target, kind)?
            .into_payload();
        Some(match payload {
            PendingRenderingUpdatePayload::DocumentScrollEvents => {
                self.dispatch_authorized_document_scroll_events(scope, host_ptr, target)
            }
            PendingRenderingUpdatePayload::AnimationStartScan(event_target) => {
                self.dispatch_authorized_animation_start_scan(scope, host_ptr, target, event_target)
            }
            PendingRenderingUpdatePayload::PostParseAutofocus => {
                self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, target)
            }
            PendingRenderingUpdatePayload::FontFaceSetReady(cycle) => {
                self.dispatch_authorized_font_face_set_ready(scope, target, cycle)
            }
        })
    }

    fn dispatch_authorized_font_face_set_ready(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        cycle: crate::script_vm::web_fonts::DocumentWebFontLoadCycleId,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        if resolved.document_handle != self.document_handle()
            || self.document_web_font_ready_layout_task_needed() != Some(cycle)
        {
            return false;
        }
        let viewport = self.layout_viewport_for_document(resolved.document_handle);
        let layout = self.with_fresh_layout_pass_for_document(
            resolved.document_handle,
            moli_layout::LayoutPassRequest::new(
                viewport,
                moli_layout::LayoutFlushReason::FontLoading,
            ),
            |_| Ok(()),
        );
        if !matches!(layout, Ok(Some(())))
            || !self.complete_document_web_font_cycle_after_layout(cycle)
        {
            return false;
        }

        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        crate::native_bridge::document::settle_document_font_face_set_load_cycle_for_document(
            scope,
            resolved.document_handle,
            cycle,
        );
        dispatch_scope.restore(scope, previous_scope);
        true
    }

    pub(crate) fn discard_pending_rendering_update_task(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        self.rendering_updates.remove(task_id).is_some()
    }

    fn dispatch_authorized_animation_start_scan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        event_target: EventTargetHandle,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let dispatched = super::super::element::dispatch_animation_start_scan(
            scope,
            host_ptr,
            resolved.document_handle,
            event_target,
        );
        dispatch_scope.restore(scope, previous_scope);
        dispatched
    }

    fn dispatch_authorized_post_parse_autofocus(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let focused = super::super::element::process_post_parse_autofocus(scope, host_ptr);
        dispatch_scope.restore(scope, previous_scope);
        focused
    }

    /// Apply an already-authorized Document rendering update. Realm lookup is
    /// resolution only: exact current/stale arbitration happened before this
    /// method was called.
    fn dispatch_authorized_document_scroll_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        self.dispatch_authorized_document_scroll_events_in_current_context(
            scope,
            host_ptr,
            target,
            resolved.document_handle,
        )
    }

    fn dispatch_authorized_document_scroll_events_in_current_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        document_handle: crate::document_runtime::DomHandle,
    ) -> bool {
        if !crate::window_host::dispatch_document_event_for_handle(
            scope,
            host_ptr,
            document_handle,
            "scroll",
        ) {
            return false;
        }

        // A scroll handler can replace its Document. Never retarget the
        // already-pending `scrollend` entry to that replacement.
        if self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            let _ = crate::window_host::dispatch_document_event_for_handle(
                scope,
                host_ptr,
                document_handle,
                "scrollend",
            );
        }
        true
    }
}

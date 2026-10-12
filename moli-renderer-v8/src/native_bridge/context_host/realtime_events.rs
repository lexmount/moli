use super::{
    JsContextHost, WindowDocumentOwner, WindowDocumentTaskTarget,
    rendering_updates::PendingRenderingUpdatePayload,
    window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask},
};
use crate::{
    document_runtime::{DomHandle, EventTargetHandle},
    page_task_queue::RendererPageRenderingUpdateTaskKind,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RealtimeEventPayload {
    DocumentScroll { end: bool },
    ElementScroll { element: DomHandle, end: bool },
    PostParseAutofocus,
    AnimationStartScan(EventTargetHandle),
}

impl RealtimeEventPayload {
    fn follows_document_open(self) -> bool {
        matches!(
            self,
            Self::DocumentScroll { .. } | Self::ElementScroll { .. }
        )
    }
}

pub(super) type RealtimeRenderingEvents =
    ExactWindowDocumentTaskLedger<u64, RendererPageRenderingUpdateTaskKind, RealtimeEventPayload>;

type PendingEvent =
    PendingExactWindowDocumentTask<u64, RendererPageRenderingUpdateTaskKind, RealtimeEventPayload>;

/// An owned phase entry. No ledger borrow or page callback crosses the take.
pub(crate) struct PreparedRealtimeRenderingEvent(PendingEvent);

impl JsContextHost {
    pub(super) fn queue_realtime_rendering_update(
        &self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
        payload: PendingRenderingUpdatePayload,
    ) -> bool {
        match payload {
            PendingRenderingUpdatePayload::DocumentScrollEvents => self.queue_realtime_events(
                target,
                kind,
                &[
                    RealtimeEventPayload::DocumentScroll { end: false },
                    RealtimeEventPayload::DocumentScroll { end: true },
                ],
            ),
            PendingRenderingUpdatePayload::PostParseAutofocus => self.queue_realtime_events(
                target,
                kind,
                &[RealtimeEventPayload::PostParseAutofocus],
            ),
            PendingRenderingUpdatePayload::AnimationStartScan(target_handle) => self
                .queue_realtime_events(
                    target,
                    kind,
                    &[RealtimeEventPayload::AnimationStartScan(target_handle)],
                ),
            _ => unreachable!("environment notifications have their own Realtime residence"),
        }
    }

    pub(crate) fn queue_realtime_element_scroll_events(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
    ) {
        if self.realtime_layout_state.is_none() {
            return;
        }
        let Some(target) = self.window_document_task_target_for_node(scope, element) else {
            return;
        };
        self.queue_realtime_events(
            target,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            &[
                RealtimeEventPayload::ElementScroll {
                    element,
                    end: false,
                },
                RealtimeEventPayload::ElementScroll { element, end: true },
            ],
        );
    }

    fn queue_realtime_events(
        &self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
        payloads: &[RealtimeEventPayload],
    ) -> bool {
        let Some(state) = &self.realtime_layout_state else {
            return false;
        };
        if !self.page_rendering_update_sender().is_open() {
            state.rendering_events.borrow_mut().retain(|_| false);
            return false;
        }
        {
            let mut events = state.rendering_events.borrow_mut();
            for payload in payloads {
                if events
                    .find_slot_index(target, kind, |pending| pending == payload)
                    .is_none()
                {
                    let id = events.allocate_task_id(|id| id);
                    events.push(PendingEvent::new(id, target, kind, *payload));
                }
            }
        }
        // The events are semantic demand. Only FrameLifecycle owns a Page
        // source entry, so the old immediate FIFO cannot deliver them twice.
        self.refresh_realtime_rendering_opportunity();
        true
    }

    pub(crate) fn has_realtime_rendering_events(&self) -> bool {
        self.realtime_layout_state.as_ref().is_some_and(|state| {
            state.rendering_events.borrow().iter().any(|pending| {
                let target = pending.target();
                self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            })
        })
    }

    pub(crate) fn take_realtime_rendering_events(
        &self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> Vec<PreparedRealtimeRenderingEvent> {
        let Some(state) = &self.realtime_layout_state else {
            return Vec::new();
        };
        // Like Blink's ScriptedAnimationController::DispatchEvents, remove
        // the entire filtered batch before any callback can enqueue more.
        state
            .rendering_events
            .borrow_mut()
            .take_where(|pending| pending.target() == target && pending.kind() == kind)
            .into_iter()
            .map(PreparedRealtimeRenderingEvent)
            .collect()
    }

    pub(crate) fn restore_realtime_rendering_events(
        &self,
        target: WindowDocumentTaskTarget,
        uncalled: Vec<PreparedRealtimeRenderingEvent>,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        if !self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) || !self.page_rendering_update_sender().is_open()
        {
            return;
        }
        let mut events = state.rendering_events.borrow_mut();
        for PreparedRealtimeRenderingEvent(pending) in uncalled.into_iter().rev() {
            if pending.target() != target && !pending.payload().follows_document_open() {
                continue;
            }
            // Retain the older uncalled entry's place ahead of newer work.
            if let Some(index) = events.find_slot_index(target, pending.kind(), |payload| {
                payload == pending.payload()
            }) {
                events.remove_at(index);
            }
            events.prepend(PendingEvent::new(
                pending.task_id(),
                target,
                pending.kind(),
                pending.into_payload(),
            ));
        }
    }

    pub(crate) fn dispatch_realtime_rendering_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        PreparedRealtimeRenderingEvent(pending): PreparedRealtimeRenderingEvent,
    ) -> bool {
        // The driver obtains `target` from this running batch's Document
        // list. Only the explicit document.open transaction may rebind it.
        if !self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) || (pending.target() != target && !pending.payload().follows_document_open())
        {
            return false;
        }
        let (element, end) = match pending.into_payload() {
            RealtimeEventPayload::PostParseAutofocus => {
                return self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, target);
            }
            RealtimeEventPayload::AnimationStartScan(event_target) => {
                return self.dispatch_authorized_animation_start_scan(
                    scope,
                    host_ptr,
                    target,
                    event_target,
                );
            }
            RealtimeEventPayload::DocumentScroll { end } => (None, end),
            RealtimeEventPayload::ElementScroll { element, end } => (Some(element), end),
        };
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous_scope = target.dispatch_scope().enter(scope);
        let name = if end { "scrollend" } else { "scroll" };
        let dispatched = if let Some(element) = element {
            super::super::element::construct_simple_event(scope, name, false, false, false)
                .is_some_and(|event| {
                    self.dispatch_public_event_best_effort(
                        scope,
                        host_ptr,
                        EventTargetHandle::Node(element),
                        event,
                        "realtime element scroll",
                    )
                    .is_ok()
                })
        } else {
            crate::window_host::dispatch_document_event_for_handle(
                scope,
                host_ptr,
                resolved.document_handle,
                name,
            )
        };
        target.dispatch_scope().restore(scope, previous_scope);
        dispatched
    }

    pub(super) fn rebind_realtime_rendering_events(
        &self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let mut events = state.rendering_events.borrow_mut();
        for pending in events.take_where(|pending| pending.target().owner() == retired) {
            // document.open preserves the Web Document's pending scroll
            // events. Parser completion and compatibility animation scans
            // belong to the retired content and must not be transferred.
            if pending.payload().follows_document_open() {
                events.push(PendingEvent::new(
                    pending.task_id(),
                    WindowDocumentTaskTarget::new(current, pending.target().dispatch_scope()),
                    pending.kind(),
                    pending.into_payload(),
                ));
            }
        }
    }

    pub(super) fn prune_realtime_rendering_events(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        state.rendering_events.borrow_mut().retain(|pending| {
            let target = pending.target();
            self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            )
        });
    }
}

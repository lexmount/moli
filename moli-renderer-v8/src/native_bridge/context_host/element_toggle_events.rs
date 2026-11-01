use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    page_task_queue::{
        RendererPageElementToggleEventCancellation, RendererPageElementToggleEventData,
        RendererPageElementToggleEventKind, RendererPageElementToggleEventState,
        RendererPageElementToggleEventTaskId,
    },
};

#[derive(Clone, Debug)]
pub(super) struct PendingElementToggleEventPayload {
    element: DomHandle,
    original_old_state: RendererPageElementToggleEventState,
    cancellation: RendererPageElementToggleEventCancellation,
}

pub(super) type ElementToggleEventState = ExactWindowDocumentTaskLedger<
    RendererPageElementToggleEventTaskId,
    RendererPageElementToggleEventKind,
    PendingElementToggleEventPayload,
>;

impl JsContextHost {
    /// Queue one element `toggle` task on the shared DOM-manipulation source.
    ///
    /// Repeated changes to the same element cancel the old closure and append a
    /// replacement at the source tail while retaining the first `oldState`, as
    /// required by the details and popover algorithms. A closed stable route
    /// retires the local coalescing slot and never falls back to a timer.
    pub(crate) fn queue_element_toggle_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        kind: RendererPageElementToggleEventKind,
        element: DomHandle,
        old_state: RendererPageElementToggleEventState,
        new_state: RendererPageElementToggleEventState,
        source: Option<DomHandle>,
    ) -> bool {
        let Some(target) = self.window_document_task_target_for_node(scope, element) else {
            return false;
        };
        let existing_index = self
            .element_toggle_events
            .find_slot_index(target, kind, |pending| pending.element == element);
        let original_old_state = existing_index
            .map(|index| {
                self.element_toggle_events
                    .at(index)
                    .payload()
                    .original_old_state
            })
            .unwrap_or(old_state);
        let task_id = self
            .element_toggle_events
            .allocate_task_id(RendererPageElementToggleEventTaskId::from_raw);
        let cancellation = RendererPageElementToggleEventCancellation::new();
        let data =
            RendererPageElementToggleEventData::new(element, original_old_state, new_state, source);

        if self
            .page_element_toggle_event_sender()
            .send(target, task_id, kind, data, cancellation.clone())
            .is_err()
        {
            if let Some(index) = existing_index {
                self.element_toggle_events
                    .remove_at(index)
                    .into_payload()
                    .cancellation
                    .cancel();
            }
            tracing::debug!(
                ?target,
                ?task_id,
                ?kind,
                element = element.index(),
                "retired element toggle event after DOM-manipulation route closure"
            );
            return false;
        }

        let pending = PendingExactWindowDocumentTask::new(
            task_id,
            target,
            kind,
            PendingElementToggleEventPayload {
                element,
                original_old_state,
                cancellation,
            },
        );
        if let Some(index) = existing_index {
            self.element_toggle_events
                .replace(index, pending)
                .into_payload()
                .cancellation
                .cancel();
        } else {
            self.element_toggle_events.push(pending);
        }
        true
    }

    pub(crate) fn cancel_element_toggle_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        kind: RendererPageElementToggleEventKind,
        element: DomHandle,
    ) -> bool {
        let Some(target) = self.window_document_task_target_for_node(scope, element) else {
            return false;
        };
        let Some(index) = self
            .element_toggle_events
            .find_slot_index(target, kind, |pending| pending.element == element)
        else {
            return false;
        };
        self.element_toggle_events
            .remove_at(index)
            .into_payload()
            .cancellation
            .cancel();
        true
    }

    pub(crate) fn current_pending_element_toggle_event_task(
        &self,
        task_id: RendererPageElementToggleEventTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageElementToggleEventKind)> {
        let pending = self.element_toggle_events.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_element_toggle_event_for_exact_target(
        &mut self,
        task_id: RendererPageElementToggleEventTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageElementToggleEventKind,
    ) -> bool {
        self.element_toggle_events
            .remove_exact(task_id, target, kind)
            .is_some()
    }

    pub(crate) fn discard_pending_element_toggle_event_task(
        &mut self,
        task_id: RendererPageElementToggleEventTaskId,
    ) -> bool {
        self.element_toggle_events
            .remove(task_id)
            .is_some_and(|pending| {
                pending.into_payload().cancellation.cancel();
                true
            })
    }

    /// Dispatch data from a task already authorized against its exact Document.
    pub(crate) fn dispatch_authorized_element_toggle_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        data: RendererPageElementToggleEventData,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let source = data
            .source()
            .and_then(|handle| crate::util::node_wrapper_from_handle(scope, handle).map(Into::into))
            .unwrap_or_else(|| v8::null(scope).into());
        let dispatched = super::super::element::construct_toggle_event(
            scope,
            "toggle",
            data.old_state().as_str(),
            data.new_state().as_str(),
            false,
            source,
        )
        .is_some_and(|event| {
            // Listener exceptions are reported at the event boundary. They do
            // not cancel the host task or suppress its microtask checkpoint.
            let _ = super::super::element::dispatch_public_event(
                scope,
                host_ptr,
                data.element(),
                event,
            );
            true
        });
        dispatch_scope.restore(scope, previous_scope);
        dispatched
    }
}

use crate::{
    context_bootstrap::{DataTransferStringCallbackTask, DataTransferStringCallbackTaskEffect},
    document_runtime::DomHandle,
    page_task_queue::{
        PageUserInteractionBodyEffect, RendererPageUserInteractionEventKind,
        RendererPageUserInteractionTaskId, RendererPageUserInteractionTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(crate) enum PendingUserInteractionTaskPayload {
    EventTarget(DomHandle),
    DataTransferString(DataTransferStringCallbackTask),
}

pub(super) type UserInteractionTaskState = ExactWindowDocumentTaskLedger<
    RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
    PendingUserInteractionTaskPayload,
>;

impl JsContextHost {
    /// Queue one event on the stable HTML user-interaction task source.
    ///
    /// The event target determines exact Document ownership. A Document
    /// selectionchange requested by a text control is retargeted to that
    /// control's owner Document before the immutable target is captured.
    pub(crate) fn queue_user_interaction_event_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        event_kind: RendererPageUserInteractionEventKind,
        requested_target: DomHandle,
    ) -> bool {
        let event_target = if matches!(
            event_kind,
            RendererPageUserInteractionEventKind::DocumentSelectionChange
        ) {
            self.dom_host()
                .owner_document_handle(requested_target)
                .unwrap_or(requested_target)
        } else {
            requested_target
        };
        let Some(target) = self.window_document_task_target_for_node(scope, event_target) else {
            return false;
        };
        let kind = RendererPageUserInteractionTaskKind::Event(event_kind);
        if event_kind.coalesces()
            && self
                .user_interaction_tasks
                .find_slot_index(target, kind, |payload| {
                    matches!(
                        payload,
                        PendingUserInteractionTaskPayload::EventTarget(pending_target)
                            if *pending_target == event_target
                    )
                })
                .is_some()
        {
            return true;
        }
        self.publish_user_interaction_task(
            target,
            kind,
            PendingUserInteractionTaskPayload::EventTarget(event_target),
        )
    }

    /// Queue one typed `DataTransferItem.getAsString` callback on the calling
    /// Realm's exact Window/Document user-interaction source.
    pub(crate) fn queue_data_transfer_string_callback_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackFunction,
        value: String,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let callback = DataTransferStringCallbackTask::new(scope, self, callback, value);
        self.publish_user_interaction_task(
            target,
            RendererPageUserInteractionTaskKind::DataTransferGetAsString,
            PendingUserInteractionTaskPayload::DataTransferString(callback),
        )
    }

    fn publish_user_interaction_task(
        &mut self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageUserInteractionTaskKind,
        payload: PendingUserInteractionTaskPayload,
    ) -> bool {
        let task_id = self
            .user_interaction_tasks
            .allocate_task_id(RendererPageUserInteractionTaskId::from_raw);
        if self
            .page_user_interaction_sender()
            .send(target, task_id, kind)
            .is_err()
        {
            tracing::debug!(
                ?target,
                ?task_id,
                ?kind,
                "retired user-interaction task after stable route closure"
            );
            return false;
        }
        self.user_interaction_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        true
    }

    pub(crate) fn current_pending_user_interaction_task(
        &self,
        task_id: RendererPageUserInteractionTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageUserInteractionTaskKind,
    )> {
        let pending = self.user_interaction_tasks.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_user_interaction_task_for_exact_target(
        &mut self,
        task_id: RendererPageUserInteractionTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageUserInteractionTaskKind,
    ) -> Option<PendingUserInteractionTaskPayload> {
        self.user_interaction_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_user_interaction_task(
        &mut self,
        task_id: RendererPageUserInteractionTaskId,
    ) -> bool {
        self.user_interaction_tasks.remove(task_id).is_some()
    }

    /// Dispatch one task already authorized against its exact Document.
    pub(crate) fn dispatch_authorized_user_interaction_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        kind: RendererPageUserInteractionTaskKind,
        payload: PendingUserInteractionTaskPayload,
    ) -> anyhow::Result<PageUserInteractionBodyEffect> {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return Ok(PageUserInteractionBodyEffect::NotApplied);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match (kind, payload) {
            (
                RendererPageUserInteractionTaskKind::Event(event_kind),
                PendingUserInteractionTaskPayload::EventTarget(event_target),
            ) => {
                let (event_type, bubbles, composed) = match event_kind {
                    RendererPageUserInteractionEventKind::DocumentSelectionChange => {
                        ("selectionchange", false, false)
                    }
                    RendererPageUserInteractionEventKind::TextControlSelectionChange => {
                        ("selectionchange", true, false)
                    }
                    RendererPageUserInteractionEventKind::TextControlSelect => {
                        ("select", true, false)
                    }
                    RendererPageUserInteractionEventKind::DialogClose => ("close", false, false),
                };
                if super::super::element::construct_simple_event(
                    scope, event_type, bubbles, false, composed,
                )
                .is_some_and(|event| {
                    let _ = super::super::element::dispatch_public_event(
                        scope,
                        host_ptr,
                        event_target,
                        event,
                    );
                    true
                }) {
                    PageUserInteractionBodyEffect::Applied
                } else {
                    PageUserInteractionBodyEffect::NotApplied
                }
            }
            (
                RendererPageUserInteractionTaskKind::DataTransferGetAsString,
                PendingUserInteractionTaskPayload::DataTransferString(callback),
            ) => match callback.invoke(scope, host_ptr) {
                DataTransferStringCallbackTaskEffect::CallbackInvoked => {
                    PageUserInteractionBodyEffect::Applied
                }
                DataTransferStringCallbackTaskEffect::CallbackNotInvoked => {
                    PageUserInteractionBodyEffect::NotApplied
                }
            },
            _ => {
                dispatch_scope.restore(scope, previous_scope);
                anyhow::bail!("user-interaction task kind did not match its Host-local payload");
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        Ok(effect)
    }
}

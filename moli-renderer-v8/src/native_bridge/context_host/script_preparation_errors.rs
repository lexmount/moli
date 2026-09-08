use crate::{
    context_bootstrap::construct_original_event,
    document_runtime::DomHandle,
    page_task_queue::{
        PageScriptPreparationErrorTargetEffect, RendererPageScriptPreparationErrorTaskId,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type ScriptPreparationErrorState =
    ExactWindowDocumentTaskLedger<RendererPageScriptPreparationErrorTaskId, (), DomHandle>;

impl JsContextHost {
    pub(crate) fn queue_script_preparation_error(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
    ) -> bool {
        let Some(target) = self.window_document_task_target_for_node(scope, element) else {
            return false;
        };
        let task_id = self
            .script_preparation_errors
            .allocate_task_id(RendererPageScriptPreparationErrorTaskId::from_raw);
        self.script_preparation_errors
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                (),
                element,
            ));
        if self
            .page_script_preparation_error_sender()
            .send(target, task_id)
            .is_ok()
        {
            return true;
        }
        let _ = self
            .script_preparation_errors
            .remove_exact(task_id, target, ());
        false
    }

    pub(crate) fn current_pending_script_preparation_error_target(
        &self,
        task_id: RendererPageScriptPreparationErrorTaskId,
    ) -> Option<WindowDocumentTaskTarget> {
        let pending = self.script_preparation_errors.pending(task_id)?;
        self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )
    }

    pub(crate) fn take_pending_script_preparation_error(
        &mut self,
        task_id: RendererPageScriptPreparationErrorTaskId,
        target: WindowDocumentTaskTarget,
    ) -> Option<DomHandle> {
        self.script_preparation_errors
            .remove_exact(task_id, target, ())
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_script_preparation_error_task(
        &mut self,
        task_id: RendererPageScriptPreparationErrorTaskId,
    ) -> bool {
        self.script_preparation_errors.remove(task_id).is_some()
    }

    pub(crate) fn dispatch_authorized_script_preparation_error(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        element: DomHandle,
    ) -> PageScriptPreparationErrorTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageScriptPreparationErrorTargetEffect::CurrentOwnerHadNoEventTarget;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        // This is a trusted plain Event, not an ErrorEvent. Construct it from
        // the intrinsic even if author code has replaced window.Event.
        let dispatched = construct_original_event(scope, "error").is_some_and(|event| {
            let _ = super::super::element::dispatch_public_event(scope, host_ptr, element, event);
            true
        });
        dispatch_scope.restore(scope, previous_scope);
        if dispatched {
            PageScriptPreparationErrorTargetEffect::DispatchedToCurrentOwner
        } else {
            PageScriptPreparationErrorTargetEffect::CurrentOwnerHadNoEventTarget
        }
    }
}

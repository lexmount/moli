use crate::{
    native_bridge::WindowExecutionContextIdentity,
    page_task_queue::{
        PageFontLoadingTargetEffect, RendererPageFontLoadingTaskId, RendererPageFontLoadingTaskKind,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type FontLoadingTaskState = ExactWindowDocumentTaskLedger<
    RendererPageFontLoadingTaskId,
    RendererPageFontLoadingTaskKind,
    FontLoadingCallback,
>;

/// Browser-created font algorithms retain their creation Realm, independently
/// of whichever Window is current when the Page selects the task.
pub(crate) struct FontLoadingCallback {
    identity: WindowExecutionContextIdentity,
    context: v8::Global<v8::Context>,
    callback: v8::Global<v8::Function>,
}

impl JsContextHost {
    pub(crate) fn queue_font_loading_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        kind: RendererPageFontLoadingTaskKind,
    ) -> bool {
        let Some(context) = callback.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(identity) = self.current_runtime_window_execution_context_identity(scope) else {
            return false;
        };
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let task_id = self
            .font_loading_tasks
            .allocate_task_id(RendererPageFontLoadingTaskId::from_raw);
        let payload = FontLoadingCallback {
            identity,
            context: v8::Global::new(scope, context),
            callback: v8::Global::new(scope, callback),
        };
        self.font_loading_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        if self
            .page_font_loading_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }
        let _ = self.font_loading_tasks.remove_exact(task_id, target, kind);
        false
    }

    pub(crate) fn current_pending_font_loading_task(
        &self,
        task_id: RendererPageFontLoadingTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageFontLoadingTaskKind)> {
        let pending = self.font_loading_tasks.pending(task_id)?;
        if !self.window_execution_context_identity_is_current(pending.payload().identity) {
            return None;
        }
        let current = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current, pending.kind()))
    }

    pub(crate) fn take_pending_font_loading_task_for_exact_target(
        &mut self,
        task_id: RendererPageFontLoadingTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageFontLoadingTaskKind,
    ) -> Option<FontLoadingCallback> {
        self.font_loading_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_font_loading_task(
        &mut self,
        task_id: RendererPageFontLoadingTaskId,
    ) -> bool {
        self.font_loading_tasks.remove(task_id).is_some()
    }

    pub(crate) fn dispatch_authorized_font_loading_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        _host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        task: FontLoadingCallback,
    ) -> PageFontLoadingTargetEffect {
        if !self.window_execution_context_identity_is_current(task.identity)
            || self.current_window_document_task_target_for_dispatch_scope(
                task.identity.dispatch_scope(),
            ) != Some(target)
        {
            return PageFontLoadingTargetEffect::CurrentOwnerCallbackRetired;
        }
        let context = v8::Local::new(scope, &task.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = target.dispatch_scope();
        let previous = dispatch_scope.enter(scope);
        let callback = v8::Local::new(scope, &task.callback);
        let receiver = context.global(scope);
        // This is a browser algorithm, not an author timer or a Web IDL
        // callback. Its selected Page task owns the eventual checkpoint.
        let result =
            moli_webidl_callback::with_webidl_callback_contexts(scope, context, context, |scope| {
                crate::exception_reporting::invoke_callback_with_report(
                    scope,
                    "font loading task",
                    "font loading task threw",
                    crate::exception_reporting::CallbackExceptionLogLevel::Debug,
                    "font loading task",
                    callback,
                    receiver.into(),
                    &[],
                )
            });
        if let Err(report) = result {
            crate::host::report_window_timer_exception(
                scope,
                Some(task.identity),
                Some(callback),
                &report,
            );
        }
        // Lightweight popups share their opener's V8 context. Font promise
        // reactions at the task checkpoint still need the captured Window.
        if matches!(
            dispatch_scope,
            super::OwnerDispatchScope::LightweightPopup(_)
        ) {
            dispatch_scope.defer_restore(scope, previous);
        } else {
            dispatch_scope.restore(scope, previous);
        }
        PageFontLoadingTargetEffect::CallbackInvokedForCurrentOwner
    }
}

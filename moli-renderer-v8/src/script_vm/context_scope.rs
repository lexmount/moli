use std::pin::pin;

use anyhow::Result;

use super::ScriptVm;
#[cfg(test)]
use super::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use crate::document_runtime::DocumentRuntime;
use crate::{frame_owner_model::FrameRealmId, native_bridge::PageScriptHost};

impl ScriptVm {
    pub(super) fn settle_retired_history_traversals(&mut self) {
        loop {
            let admissions = self
                ._context_host
                .borrow_mut()
                .pending_history_traversal_admissions
                .take_retired();
            if admissions.is_empty() {
                return;
            }
            let _ = self.with_default_context_scope(|scope, _| {
                // Neither a host borrow nor native retirement stack crosses JS.
                // Reentrant retirement is extracted by the next loop iteration.
                crate::context_bootstrap::abort_history_traversal_admissions(scope, admissions);
                Self::perform_microtask_checkpoints(scope, None)
            });
        }
    }

    pub(super) fn cancel_history_traversals_for_retiring_window(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) {
        let admissions = self
            ._context_host
            .borrow_mut()
            .pending_history_traversal_admissions
            .take_for_owner(owner);
        if admissions.is_empty() {
            return;
        }
        let _ = self.with_default_context_scope(|scope, _| {
            crate::context_bootstrap::abort_history_traversal_admissions(scope, admissions);
            Ok(())
        });
    }

    pub(crate) fn start_scanned_image_preload(
        &mut self,
        request_url: url::Url,
        fetch_priority: Option<moli_fetch::FetchPriorityHint>,
    ) -> Result<crate::network_host::ScannedImagePreloadStart> {
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok(crate::network_host::start_scanned_image_preload(
                scope,
                host,
                request_url,
                fetch_priority,
            ))
        })
    }

    /// Enters an exact V8 context for one synchronous body operation.
    ///
    /// Context selection is not a browser task boundary. This helper therefore
    /// never runs a microtask checkpoint. The selected Page-task dispatcher,
    /// an explicit JS command completion, or a documented algorithm boundary
    /// must own any checkpoint required after `op` returns.
    pub(super) fn with_context_scope_by_ptr<T>(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        let context_host = self._context_host.clone();
        let result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: callers pass a context owned by this ScriptVm. The
                // non-escaping closure runs while the matching document isolate
                // is entered and ScriptVm remains exclusively borrowed.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // V8 callbacks are re-entrant. Keep only the owning Rc alive;
                // a RefCell guard here would make callbacks panic on borrow_mut.
                let runtime_ptr: *mut PageScriptHost = (*context_host).as_ptr();
                op(scope, runtime_ptr)
            });
        let publication = self.publish_prebootstrapped_isolated_window_contexts();
        result.and_then(|value| publication.map(|()| value))
    }

    /// Enters the main Window realm for one synchronous body operation.
    ///
    /// Like `with_context_scope_by_ptr`, this is deliberately body-only. Its
    /// name must not imply that returning from the Rust closure completes an
    /// HTML task or a protocol command.
    pub(super) fn with_default_context_scope<T>(
        &mut self,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        let context_ptr = self.ensure_main_world()?.context_ptr();
        self.with_context_scope_by_ptr(context_ptr, op)
    }

    /// Update an already-materialized JavaScript surface without creating one.
    pub(super) fn with_current_main_world_scope<T>(
        &mut self,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<Option<T>> {
        let Some(runtime) = self.current_main_world() else {
            return Ok(None);
        };
        self.with_context_scope_by_ptr(runtime.context_ptr(), op)
            .map(Some)
    }

    /// Script/event entry with the live native Document and an entered realm.
    pub(super) fn with_main_world_document_scope<T>(
        &mut self,
        op: impl FnOnce(
            &mut v8::PinScope<'_, '_>,
            *mut PageScriptHost,
            &mut DocumentRuntime,
        ) -> Result<T>,
    ) -> Result<T> {
        let context_ptr = self.ensure_main_world()?.context_ptr();
        self.with_document_context_scope_by_ptr(context_ptr, op)
    }

    pub(super) fn with_current_main_world_document_scope<T>(
        &mut self,
        op: impl FnOnce(
            &mut v8::PinScope<'_, '_>,
            *mut PageScriptHost,
            &mut DocumentRuntime,
        ) -> Result<T>,
    ) -> Result<Option<T>> {
        let Some(runtime) = self.current_main_world() else {
            return Ok(None);
        };
        self.with_document_context_scope_by_ptr(runtime.context_ptr(), op)
            .map(Some)
    }

    fn with_document_context_scope_by_ptr<T>(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        op: impl FnOnce(
            &mut v8::PinScope<'_, '_>,
            *mut PageScriptHost,
            &mut DocumentRuntime,
        ) -> Result<T>,
    ) -> Result<T> {
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: the caller obtains this pointer from an owned live
                // realm before borrowing the other ScriptVm fields.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                op(scope, (*context_host).as_ptr(), document_runtime)
            })
    }

    /// Inspect the current realm without completing a task or running reactions.
    #[cfg(test)]
    pub(crate) fn with_default_context_scope_for_test<T>(
        &mut self,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        self.with_default_context_scope(op)
    }

    #[cfg(test)]
    pub(crate) fn lazy_constructor_materialization_count_for_test(
        &mut self,
        name: &str,
    ) -> Result<usize> {
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::lazy_constructor_materialization_count(scope, name))
        })
    }

    /// Enters the exact main/child frame realm without declaring task completion.
    pub(super) fn with_frame_realm_scope<T>(
        &mut self,
        realm_id: FrameRealmId,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        let context_ptr = self.frame_realm_context_ptr(realm_id)?;
        self.with_context_scope_by_ptr(context_ptr, op)
    }

    /// Compatibility primitive for low-level tests that intentionally model a
    /// complete synchronous JS turn without going through a Page dispatcher.
    /// Product code must use a named task/command/algorithm completion instead.
    #[cfg(test)]
    pub(super) fn with_context_scope_by_ptr_and_checkpoint_for_test<T>(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        self.with_context_scope_by_ptr(context_ptr, |scope, runtime_ptr| {
            let result = op(scope, runtime_ptr);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            result
        })
    }

    #[cfg(test)]
    pub(super) fn with_default_context_scope_and_checkpoint_for_test<T>(
        &mut self,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        self.with_default_context_scope(|scope, runtime_ptr| {
            let result = op(scope, runtime_ptr);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            result
        })
    }

    #[cfg(test)]
    pub(super) fn with_frame_realm_scope_and_checkpoint_for_test<T>(
        &mut self,
        realm_id: FrameRealmId,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut PageScriptHost) -> Result<T>,
    ) -> Result<T> {
        self.with_frame_realm_scope(realm_id, |scope, runtime_ptr| {
            let result = op(scope, runtime_ptr);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            result
        })
    }
}

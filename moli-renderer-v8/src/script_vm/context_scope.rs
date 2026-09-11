use std::pin::pin;

use anyhow::Result;

use super::ScriptVm;
#[cfg(test)]
use super::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use crate::{frame_owner_model::FrameRealmId, native_bridge::JsContextHost};

impl ScriptVm {
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
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
    ) -> Result<T> {
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
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
                let runtime_ptr: *mut JsContextHost = (*context_host).as_ptr();
                op(scope, runtime_ptr)
            })
    }

    /// Enters the main Window realm for one synchronous body operation.
    ///
    /// Like `with_context_scope_by_ptr`, this is deliberately body-only. Its
    /// name must not imply that returning from the Rust closure completes an
    /// HTML task or a protocol command.
    pub(crate) fn with_default_context_scope<T>(
        &mut self,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
    ) -> Result<T> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.with_context_scope_by_ptr(context_ptr, op)
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
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
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
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
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
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
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
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
    ) -> Result<T> {
        self.with_frame_realm_scope(realm_id, |scope, runtime_ptr| {
            let result = op(scope, runtime_ptr);
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            result
        })
    }
}

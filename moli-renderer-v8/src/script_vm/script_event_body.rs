//! Body-only dispatch for script-element terminal events and internal Window errors.
//!
//! These primitives may enter author code, but they never decide that an HTML
//! task or an algorithm step has ended. Selected Page tasks and the few
//! synchronous parser/module/runtime algorithms that still need a checkpoint
//! consume them through their own named completion boundary.

use std::pin::pin;

use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::context_bootstrap::{
    ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT, dispatch_window_error_event_with_details,
};
use crate::host::ScriptEventTask;
use crate::native_bridge::JsContextHost;
use crate::util::{get_private_value, v8_string, v8str};

impl ScriptVm {
    pub(crate) fn dispatch_script_event_body_best_effort(&mut self, task: &ScriptEventTask) {
        if let Err(error) = self.dispatch_script_event_body(task) {
            self.record_runtime_warning(format_args!(
                "script {} body dispatch failed for `{}`: {error}",
                task.event_name(),
                task.handle
            ));
        }
    }

    pub(crate) fn dispatch_script_event_body(&mut self, task: &ScriptEventTask) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime
                    .host_dispatch_script_event(scope, host_ptr, task)
                    .map_err(anyhow::Error::msg)
            })
    }

    pub(crate) fn report_window_error_body_best_effort(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) {
        if let Err(error) = self.report_window_error_body(message, filename, error_constructor) {
            self.record_runtime_warning(format_args!(
                "window script failure body dispatch failed for `{}`: {error}",
                filename.unwrap_or("")
            ));
        }
    }

    pub(crate) fn report_window_error_body(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let global = scope.get_current_context().global(scope);
                let message_value = v8_string(scope, message)
                    .ok_or_else(|| anyhow!("failed to allocate reportError message"))?;
                let error_value = window_script_failure_error_value(
                    scope,
                    global,
                    error_constructor,
                    message_value,
                );
                if let Some(filename) = filename
                    && let Some(filename_value) = v8_string(scope, filename)
                    && let Ok(error_object) = v8::Local::<v8::Object>::try_from(error_value)
                {
                    let _ = error_object.set(
                        scope,
                        v8str(scope, "fileName").into(),
                        filename_value.into(),
                    );
                }
                // Internal script failure reporting must not call the page-visible
                // window.reportError function while lifecycle state is unwinding.
                // Dispatch the equivalent ErrorEvent body directly.
                // SAFETY: as_ptr() — V8 callbacks are re-entrant; borrow_mut() panics. See util.rs.
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                dispatch_window_error_event_with_details(
                    scope,
                    host_ptr,
                    message,
                    filename.unwrap_or(""),
                    0,
                    0,
                    Some(error_value),
                )
                .map_err(anyhow::Error::msg)
            })
    }
}

fn window_script_failure_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    message: v8::Local<'s, v8::String>,
) -> v8::Local<'s, v8::Value> {
    let constructor = match error_constructor {
        Some(crate::types::ScriptErrorConstructorKind::WebAssemblyCompileError) => {
            original_webassembly_error_constructor(
                scope,
                global,
                ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
            )
        }
        Some(crate::types::ScriptErrorConstructorKind::WebAssemblyLinkError) => {
            original_webassembly_error_constructor(
                scope,
                global,
                ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
            )
        }
        _ => None,
    };
    constructor
        .and_then(|constructor| constructor.new_instance(scope, &[message.into()]))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| match error_constructor {
            Some(crate::types::ScriptErrorConstructorKind::SyntaxError) => {
                v8::Exception::syntax_error(scope, message)
            }
            Some(crate::types::ScriptErrorConstructorKind::TypeError) => {
                v8::Exception::type_error(scope, message)
            }
            _ => v8::Exception::error(scope, message),
        })
}

fn original_webassembly_error_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    original_slot: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    get_private_value(scope, global, original_slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

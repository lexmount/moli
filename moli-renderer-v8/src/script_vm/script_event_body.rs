//! Body-only dispatch for script-element terminal events and internal Window errors.
//!
//! These primitives may enter author code, but they never decide that an HTML
//! task or an algorithm step has ended. Selected Page tasks and the few
//! synchronous parser/module/runtime algorithms that still need a checkpoint
//! consume them through their own named completion boundary.

use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::context_bootstrap::{
    ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT, dispatch_window_error_event_with_details,
};
use crate::frame_owner_model::{FrameDocumentTaskOwner, FrameRealmId};
use crate::host::ScriptEventTask;
use crate::module_runtime::{ModuleEntryId, ModuleLoadError};
use crate::native_bridge::PageScriptHost;
use crate::types::ScriptErrorValue;
use crate::util::{get_private_value, v8_string, v8str};

impl ScriptVm {
    pub(super) fn accept_ready_runtime_script_events(&mut self) {
        let host = &self._context_host;
        self.document_runtime
            .accept_ready_runtime_script_events(|url, message| {
                host.borrow().emit_script_load_failure(url, message);
            });
    }

    pub(crate) fn emit_script_load_failure(&self, url: &str, message: &str) {
        self._context_host
            .borrow()
            .emit_script_load_failure(url, message);
    }

    pub(crate) fn emit_module_script_load_failure(
        &self,
        entry_url: &url::Url,
        error: &ModuleLoadError,
    ) {
        if let Some(resource) = error.failed_resource() {
            self._context_host.borrow().emit_diagnostic(|source| {
                Some(crate::DiagnosticEvent::script_load_failure(
                    source,
                    entry_url.as_str(),
                    resource.url().as_str(),
                    error.message(),
                ))
            });
        }
    }

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
        self.with_main_world_document_scope(|scope, host_ptr, document_runtime| {
            document_runtime
                .host_dispatch_script_event(scope, host_ptr, task)
                .map_err(anyhow::Error::msg)
        })
    }

    pub(crate) fn report_window_error_body_best_effort(
        &mut self,
        message: &str,
        filename: Option<&str>,
        error_value: Option<ScriptErrorValue>,
        failed_module: Option<ModuleEntryId>,
    ) {
        if let Err(error) =
            self.report_window_error_body(message, filename, error_value, failed_module)
        {
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
        error_value: Option<ScriptErrorValue>,
        failed_module: Option<ModuleEntryId>,
    ) -> Result<()> {
        // An evaluated module keeps the original thrown value, including a TLA
        // rejection, in its V8 module record. Read it only at the existing error
        // reporting boundary, after module evaluation has finished unwinding.
        let failed_module = failed_module
            .filter(|_| !matches!(error_value, Some(ScriptErrorValue::Retained(_))))
            .and_then(|entry| self.document_runtime.native_compiled_module(entry));
        self.with_default_context_scope(|scope, host_ptr| {
            if let Some(module) = &failed_module {
                let module = v8::Local::new(scope, module);
                if module.get_status() == v8::ModuleStatus::Errored {
                    let reason = module.get_exception();
                    let (message, source, lineno, colno) =
                        crate::exception_reporting::exception_message_details(scope, reason);
                    let filename = if source.is_empty() {
                        filename.unwrap_or_default()
                    } else {
                        &source
                    };
                    return dispatch_window_error_event_with_details(
                        scope,
                        host_ptr,
                        &message,
                        filename,
                        lineno,
                        colno,
                        Some(reason),
                    )
                    .map_err(anyhow::Error::msg);
                }
            }
            dispatch_script_failure_error_body(scope, host_ptr, message, filename, error_value)
        })
    }

    pub(crate) fn report_child_window_error_body(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        message: &str,
        filename: &str,
        error_value: Option<ScriptErrorValue>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.child_parser_module_route_task_is_current(owner, realm_id),
            "child module exception reporting owner is no longer current"
        );
        self.with_frame_realm_scope(realm_id, |scope, host_ptr| {
            dispatch_script_failure_error_body(
                scope,
                host_ptr,
                message,
                Some(filename),
                error_value,
            )
        })
    }
}

fn dispatch_script_failure_error_body(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    message: &str,
    filename: Option<&str>,
    error_value: Option<ScriptErrorValue>,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    let message_value = v8_string(scope, message)
        .ok_or_else(|| anyhow!("failed to allocate reportError message"))?;
    let retained = matches!(error_value, Some(ScriptErrorValue::Retained(_)));
    let error_value = match error_value {
        Some(ScriptErrorValue::Retained(id)) => {
            super::native_module::retained_module_exception(scope, id)?
        }
        Some(ScriptErrorValue::Constructor(kind)) => {
            window_script_failure_error_value(scope, global, Some(kind), message_value)
        }
        None => window_script_failure_error_value(scope, global, None, message_value),
    };
    // Retained V8 exceptions already carry engine-owned source information.
    // Read that metadata, not author-visible stack or location properties, and
    // do not replace a dependency's URL with the root script's fallback URL.
    let location = retained.then(|| {
        let exception_message = v8::Exception::create_message(scope, error_value);
        let filename = exception_message
            .get_script_resource_name(scope)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let line = exception_message
            .get_line_number(scope)
            .and_then(|line| u32::try_from(line).ok())
            .unwrap_or(0);
        let column = exception_message
            .get_start_column()
            .checked_add(1)
            .and_then(|column| u32::try_from(column).ok())
            .unwrap_or(0);
        (filename, line, column)
    });
    // Location metadata belongs to the ErrorEvent. Never mutate
    // the original exception (or invoke an author's setter).
    if !retained
        && let Some(filename) = filename
        && let Some(filename_value) = v8_string(scope, filename)
        && let Ok(error_object) = v8::Local::<v8::Object>::try_from(error_value)
    {
        let _ = error_object.set(
            scope,
            v8str(scope, "fileName").into(),
            filename_value.into(),
        );
    }
    // This body must not call the page-visible reportError function or own
    // a checkpoint, whether it runs in the main Window or a child realm.
    dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        message,
        location
            .as_ref()
            .and_then(|(filename, _, _)| filename.as_deref())
            .or(filename)
            .unwrap_or(""),
        location.as_ref().map_or(0, |(_, line, _)| *line),
        location.as_ref().map_or(0, |(_, _, column)| *column),
        Some(error_value),
    )
    .map_err(anyhow::Error::msg)
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

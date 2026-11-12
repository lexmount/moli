use std::{ptr::NonNull, rc::Rc};

use crate::{
    callback_invocation::{
        SynchronousWebIdlCallbackOutcome, invoke_synchronous_webidl_callback_interface,
    },
    context_bootstrap::dispatch_window_error_event_with_details,
    exception_reporting::{CallbackExceptionLogLevel, V8ExceptionReport, log_callback_exception},
    native_bridge::{PageScriptHost, WindowExecutionContextIdentity},
    util::context_host_ptr_from_global_bridge,
    webidl,
};
use moli_xpath::NamespaceResolver;

/// A synchronous XPath parser adapter over one prepared Web IDL callback.
///
/// `moli-xpath` may clone its resolver while parsing, so the independently
/// rooted callback snapshot is shared with `Rc`. The raw scope pointer is safe
/// only because the entire adapter remains inside one `Document.evaluate` /
/// `XPathEvaluator.evaluate` / `createExpression` V8 callback and never crosses an await, task, or
/// thread boundary.
#[derive(Clone)]
pub(super) struct V8XPathNamespaceResolver<'s, 'i> {
    scope: NonNull<v8::PinScope<'s, 'i>>,
    callback: Rc<webidl::PreparedWebIdlCallbackInterface>,
    host_ptr: Option<*mut PageScriptHost>,
    execution_context: Option<WindowExecutionContextIdentity>,
}

impl<'s, 'i> V8XPathNamespaceResolver<'s, 'i> {
    pub(super) fn new(
        scope: &mut v8::PinScope<'s, 'i>,
        callback: webidl::WebIdlCallbackInterface,
    ) -> Self {
        let callback = callback.prepare(scope);
        let relevant_context = callback.relevant_context(scope);
        let host_ptr = {
            let scope = &mut v8::ContextScope::new(scope, relevant_context);
            context_host_ptr_from_global_bridge(scope)
        };
        let execution_context = host_ptr.and_then(|host_ptr| {
            unsafe { &*host_ptr }
                .window_execution_context_identity_for_v8_context(scope, relevant_context)
        });
        Self {
            scope: NonNull::from(scope),
            callback: Rc::new(callback),
            host_ptr,
            execution_context,
        }
    }

    fn execution_context_is_current(&self) -> bool {
        match (self.host_ptr, self.execution_context) {
            (Some(host_ptr), Some(identity)) => {
                unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
            }
            _ => true,
        }
    }

    fn report_exception(&self, outer_scope: &mut v8::PinScope<'_, '_>, report: &V8ExceptionReport) {
        log_callback_exception(
            CallbackExceptionLogLevel::Debug,
            "XPath namespace resolver callback threw",
            "lookupNamespaceURI",
            report,
        );
        if !self.execution_context_is_current() {
            return;
        }
        let Some(host_ptr) = self.host_ptr else {
            return;
        };
        let relevant_context = self.callback.relevant_context(outer_scope);
        let scope = &mut v8::ContextScope::new(outer_scope, relevant_context);
        let error_value = report
            .exception
            .as_ref()
            .map(|exception| v8::Local::new(scope, exception));
        let _ = dispatch_window_error_event_with_details(
            scope,
            host_ptr,
            &report.summary,
            report.source.as_deref().unwrap_or(""),
            report.line.unwrap_or(0) as u32,
            report.column.unwrap_or(0) as u32,
            error_value,
        );
    }
}

impl NamespaceResolver for V8XPathNamespaceResolver<'_, '_> {
    fn resolve_namespace_prefix(&self, prefix: &str) -> Option<String> {
        if !self.execution_context_is_current() {
            return None;
        }
        let outer_scope = unsafe { self.scope.as_ptr().as_mut()? };
        let prefix = v8::String::new(outer_scope, prefix)?;
        let callback_this = v8::undefined(outer_scope).into();
        let arguments = [prefix.into()];
        match invoke_synchronous_webidl_callback_interface(
            outer_scope,
            &self.callback,
            callback_this,
            "lookupNamespaceURI",
            &arguments,
            |scope, value| {
                if value.is_null_or_undefined() {
                    return Some(None);
                }
                match webidl::convert::<webidl::DomString>(
                    scope,
                    value,
                    webidl::Context::member("XPathNSResolver", "lookupNamespaceURI"),
                ) {
                    Ok(value) => Some(Some(value.0)),
                    Err(error) => {
                        webidl::throw_error(scope, &error);
                        None
                    }
                }
            },
        ) {
            SynchronousWebIdlCallbackOutcome::Returned(namespace) => namespace,
            SynchronousWebIdlCallbackOutcome::Threw(report) => {
                self.report_exception(outer_scope, &report);
                None
            }
            SynchronousWebIdlCallbackOutcome::Terminated => None,
        }
    }
}

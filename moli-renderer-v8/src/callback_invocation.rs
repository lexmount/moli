use crate::exception_reporting::{
    CallbackExceptionLogLevel, V8ExceptionReport, build_callback_context,
    build_event_handler_exception_report, invoke_callback_with_report, log_callback_exception,
};
use crate::{
    host::WINDOW_EVENT_SLOT,
    native_bridge::{JsContextHost, WindowExecutionContextIdentity},
    util::v8str,
};
use moli_webidl_callback::{
    PreparedWebIdlCallbackFunction, PreparedWebIdlCallbackInterface, WebIdlCallbackInvocation,
    WebIdlCallbackResolutionFailure, invoke_webidl_callback, invoke_webidl_callback_function,
    with_webidl_callback_contexts,
};

/// Invokes one synchronous Web IDL callback function while preserving a thrown
/// V8 exception for the calling Web API algorithm.
///
/// This helper does not report exceptions, perform a microtask checkpoint, or
/// acquire Page/Document authority. The synchronous API owner remains
/// responsible for receiver/argument construction, abrupt completion, and
/// result conversion.
pub(crate) fn invoke_synchronous_webidl_callback_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &PreparedWebIdlCallbackFunction,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Global<v8::Value>> {
    invoke_webidl_callback_function(
        scope,
        callback,
        receiver,
        arguments,
        |scope, callback, receiver, arguments| {
            callback
                .call(scope, receiver, arguments)
                .map(|value| v8::Global::new(scope, value))
                .ok_or(())
        },
    )
    .ok()
}

/// The synchronous completion of a callback-interface invocation.
///
/// `Threw` retains the exact JavaScript exception report so the calling Web API
/// can either rethrow the exception or report it before mapping the algorithm
/// to another failure. `Terminated` represents V8 termination or another abrupt
/// call that did not expose a catchable exception. This boundary itself never
/// reports or translates either outcome.
pub(crate) enum SynchronousWebIdlCallbackOutcome<R> {
    Returned(R),
    Threw(Box<V8ExceptionReport>),
    Terminated,
}

/// Invokes one prepared single-operation Web IDL callback interface.
///
/// The callback crate owns relevant/incumbent context entry and the
/// callable-versus-operation-lookup branch. The API owner supplies the
/// operation name, callable-branch receiver, arguments, and return conversion.
/// This boundary performs no exception reporting, microtask checkpoint, or
/// Page/Document scheduling.
pub(crate) fn invoke_synchronous_webidl_callback_interface<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &PreparedWebIdlCallbackInterface,
    callback_this: v8::Local<'s, v8::Value>,
    operation_name: &str,
    arguments: &[v8::Local<'s, v8::Value>],
    convert_return: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Value>) -> Option<R>,
) -> SynchronousWebIdlCallbackOutcome<R> {
    enum Failure {
        Captured(Box<V8ExceptionReport>),
        Pending,
    }

    let relevant_context = callback.relevant_context(scope);
    let incumbent_context = callback.incumbent_context(scope);
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let result =
        with_webidl_callback_contexts(&mut scope, relevant_context, incumbent_context, |scope| {
            let callback_object = callback.callback(scope);
            invoke_webidl_callback(
                scope,
                WebIdlCallbackInvocation::new(
                    callback_object,
                    callback_this,
                    callback.callable_at_conversion(),
                    operation_name,
                    arguments,
                ),
                |scope, callback, receiver, arguments| {
                    let value = callback
                        .call(scope, receiver, arguments)
                        .ok_or(Failure::Pending)?;
                    convert_return(scope, value).ok_or(Failure::Pending)
                },
                |scope, failure| {
                    Failure::Captured(Box::new(build_event_handler_exception_report(
                        scope,
                        failure.exception(),
                        failure.message(),
                        failure.stack_trace(),
                    )))
                },
            )
        });

    match result {
        Ok(value) => SynchronousWebIdlCallbackOutcome::Returned(value),
        Err(Failure::Captured(exception)) => SynchronousWebIdlCallbackOutcome::Threw(exception),
        Err(Failure::Pending) if scope.exception().is_some() => {
            let exception = scope.exception();
            let message = scope.message();
            let stack_trace = scope.stack_trace();
            SynchronousWebIdlCallbackOutcome::Threw(Box::new(build_event_handler_exception_report(
                &mut scope,
                exception,
                message,
                stack_trace,
            )))
        }
        Err(Failure::Pending) => SynchronousWebIdlCallbackOutcome::Terminated,
    }
}

pub(crate) struct CallbackInvocation<'s, 'a> {
    callback: v8::Local<'s, v8::Object>,
    callback_this: v8::Local<'s, v8::Value>,
    relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    host_ptr: Option<*mut JsContextHost>,
    is_callable: bool,
    operation_name: &'a str,
    arguments: &'a [v8::Local<'s, v8::Value>],
    current_event: Option<v8::Local<'s, v8::Object>>,
}

impl<'s, 'a> CallbackInvocation<'s, 'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        callback: v8::Local<'s, v8::Object>,
        callback_this: v8::Local<'s, v8::Value>,
        relevant_context: v8::Local<'s, v8::Context>,
        incumbent_context: v8::Local<'s, v8::Context>,
        is_callable: bool,
        operation_name: &'a str,
        arguments: &'a [v8::Local<'s, v8::Value>],
        current_event: Option<v8::Local<'s, v8::Object>>,
    ) -> Self {
        Self {
            callback,
            callback_this,
            relevant_context,
            incumbent_context,
            relevant_identity: None,
            host_ptr: None,
            is_callable,
            operation_name,
            arguments,
            current_event,
        }
    }

    pub(crate) fn with_execution_context_currentness(
        mut self,
        host_ptr: *mut JsContextHost,
        relevant_identity: Option<WindowExecutionContextIdentity>,
    ) -> Self {
        self.host_ptr = Some(host_ptr);
        self.relevant_identity = relevant_identity;
        self
    }
}

pub(crate) enum CallbackInvocationOutcome {
    Returned(v8::Global<v8::Value>),
    Threw(Box<V8ExceptionReport>),
    Retired,
}

pub(crate) struct CallbackInvoker;

impl CallbackInvoker {
    pub(crate) fn invoke<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        callback_kind: &str,
        log_label: &str,
        log_level: CallbackExceptionLogLevel,
        callback_name: &str,
        invocation: CallbackInvocation<'s, '_>,
    ) -> CallbackInvocationOutcome {
        Self::invoke_with_completion(
            scope,
            callback_kind,
            log_label,
            log_level,
            callback_name,
            invocation,
            false,
            |_scope, outcome| outcome,
        )
    }

    pub(crate) fn invoke_event_and_then<'s, R>(
        scope: &mut v8::PinScope<'s, '_>,
        callback_kind: &str,
        log_label: &str,
        log_level: CallbackExceptionLogLevel,
        callback_name: &str,
        invocation: CallbackInvocation<'s, '_>,
        complete: impl FnOnce(&mut v8::PinScope<'s, '_>, CallbackInvocationOutcome) -> R,
    ) -> R {
        let host_ptr = invocation.host_ptr;
        let event_callback_scope = host_ptr
            .map(|host_ptr| unsafe { &mut *host_ptr }.enter_event_callback_invocation_scope());
        let defer_window_event_restore = event_callback_scope
            .as_ref()
            .is_some_and(|event_scope| event_scope.is_outermost())
            && host_ptr
                .is_some_and(|host_ptr| !unsafe { &*host_ptr }.explicit_event_dispatch_is_active());
        let result = Self::invoke_with_completion(
            scope,
            callback_kind,
            log_label,
            log_level,
            callback_name,
            invocation,
            defer_window_event_restore,
            complete,
        );
        drop(event_callback_scope);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_with_completion<'s, R>(
        scope: &mut v8::PinScope<'s, '_>,
        callback_kind: &str,
        log_label: &str,
        log_level: CallbackExceptionLogLevel,
        callback_name: &str,
        invocation: CallbackInvocation<'s, '_>,
        defer_window_event_restore: bool,
        complete: impl FnOnce(&mut v8::PinScope<'s, '_>, CallbackInvocationOutcome) -> R,
    ) -> R {
        if let Some(host_ptr) = invocation.host_ptr {
            unsafe { &*host_ptr }.debug_assert_not_in_structural_mutation("callback invocation");
        }
        if let (Some(host_ptr), Some(identity)) =
            (invocation.host_ptr, invocation.relevant_identity)
            && !unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
        {
            return complete(scope, CallbackInvocationOutcome::Retired);
        }

        with_webidl_callback_contexts(
            scope,
            invocation.relevant_context,
            invocation.incumbent_context,
            |scope| {
                let relevant_dispatch_scope = invocation
                    .relevant_identity
                    .map(WindowExecutionContextIdentity::dispatch_scope);
                let previous_relevant_dispatch_scope =
                    relevant_dispatch_scope.map(|dispatch_scope| dispatch_scope.enter(scope));
                let relevant_context = invocation.relevant_context;
                let previous_window_event =
                    invocation
                        .host_ptr
                        .and(invocation.current_event)
                        .map(|event| {
                            let global = relevant_context.global(scope);
                            let event_key = v8str(scope, WINDOW_EVENT_SLOT);
                            let previous = global
                                .get(scope, event_key.into())
                                .unwrap_or_else(|| v8::undefined(scope).into());
                            let _ = global.set(scope, event_key.into(), event.into());
                            previous
                        });

                let webidl_invocation = WebIdlCallbackInvocation::new(
                    invocation.callback,
                    invocation.callback_this,
                    invocation.is_callable,
                    invocation.operation_name,
                    invocation.arguments,
                );
                let result = invoke_webidl_callback(
                    scope,
                    webidl_invocation,
                    |scope, callback, receiver, arguments| {
                        invoke_callback_with_report(
                            scope,
                            callback_kind,
                            log_label,
                            log_level,
                            callback_name,
                            callback,
                            receiver,
                            arguments,
                        )
                    },
                    |scope, failure| {
                        capture_callback_resolution_failure(
                            scope,
                            log_label,
                            log_level,
                            callback_name,
                            failure,
                        )
                    },
                );

                let outcome = match result {
                    Ok(value) => CallbackInvocationOutcome::Returned(value),
                    Err(report) => CallbackInvocationOutcome::Threw(report),
                };
                let completed = complete(scope, outcome);

                if let Some(previous) = previous_window_event {
                    if defer_window_event_restore && let Some(host_ptr) = invocation.host_ptr {
                        crate::context_bootstrap::enqueue_window_event_restore_after_microtask_checkpoint(
                            scope,
                            host_ptr,
                            invocation.relevant_identity,
                            relevant_context,
                            previous,
                        );
                    } else {
                        let global = relevant_context.global(scope);
                        let _ = global.set(scope, v8str(scope, WINDOW_EVENT_SLOT).into(), previous);
                    }
                }
                if let (Some(dispatch_scope), Some(previous_dispatch_scope)) =
                    (relevant_dispatch_scope, previous_relevant_dispatch_scope)
                {
                    dispatch_scope.restore(scope, previous_dispatch_scope);
                }
                completed
            },
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_callback_resolution_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    log_label: &str,
    log_level: CallbackExceptionLogLevel,
    callback_name: &str,
    failure: WebIdlCallbackResolutionFailure<'s, '_>,
) -> Box<V8ExceptionReport> {
    let mut captured = build_event_handler_exception_report(
        scope,
        failure.exception(),
        failure.message(),
        failure.stack_trace(),
    );
    captured.callback_context = Some(build_callback_context(
        scope,
        failure.callback().into(),
        failure.arguments(),
    ));
    log_callback_exception(log_level, log_label, callback_name, &captured);
    Box::new(captured)
}

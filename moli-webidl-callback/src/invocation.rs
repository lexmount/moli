use std::pin::pin;

use crate::PreparedWebIdlCallbackFunction;

/// The non-owning values needed to invoke one Web IDL callback.
pub struct WebIdlCallbackInvocation<'s, 'a> {
    callback: v8::Local<'s, v8::Object>,
    callback_this: v8::Local<'s, v8::Value>,
    callable_at_conversion: bool,
    operation_name: &'a str,
    arguments: &'a [v8::Local<'s, v8::Value>],
}

impl<'s, 'a> WebIdlCallbackInvocation<'s, 'a> {
    pub fn new(
        callback: v8::Local<'s, v8::Object>,
        callback_this: v8::Local<'s, v8::Value>,
        callable_at_conversion: bool,
        operation_name: &'a str,
        arguments: &'a [v8::Local<'s, v8::Value>],
    ) -> Self {
        Self {
            callback,
            callback_this,
            callable_at_conversion,
            operation_name,
            arguments,
        }
    }
}

/// A callback-resolution failure while its V8 `TryCatch` is still active.
///
/// The renderer supplies the failure consumer so it can retain its own
/// exception-reporting policy without this crate depending on Page or Window
/// types.
pub struct WebIdlCallbackResolutionFailure<'s, 'a> {
    callback: v8::Local<'s, v8::Object>,
    arguments: &'a [v8::Local<'s, v8::Value>],
    exception: Option<v8::Local<'s, v8::Value>>,
    message: Option<v8::Local<'s, v8::Message>>,
    stack_trace: Option<v8::Local<'s, v8::Value>>,
}

impl<'s, 'a> WebIdlCallbackResolutionFailure<'s, 'a> {
    pub fn callback(&self) -> v8::Local<'s, v8::Object> {
        self.callback
    }

    pub fn arguments(&self) -> &'a [v8::Local<'s, v8::Value>] {
        self.arguments
    }

    pub fn exception(&self) -> Option<v8::Local<'s, v8::Value>> {
        self.exception
    }

    pub fn message(&self) -> Option<v8::Local<'s, v8::Message>> {
        self.message
    }

    pub fn stack_trace(&self) -> Option<v8::Local<'s, v8::Value>> {
        self.stack_trace
    }
}

/// Enters the callback's relevant Realm, microtask scope, and captured incumbent
/// settings object, in that order.
///
/// The supplied operation includes operation lookup, invocation, and return
/// conversion. The incumbent scope is removed before a Scoped-policy checkpoint
/// can run on microtask-scope exit. Explicit policy leaves checkpoints to the
/// embedder; Auto policy retains V8's existing per-API-call behavior.
/// Renderer-specific ambient state, such as `window.event`, can be installed
/// inside `operation` without moving that policy into this crate.
pub fn with_webidl_callback_contexts<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
    operation: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    if scope.get_microtasks_policy() == v8::MicrotasksPolicy::Auto {
        return with_incumbent_context(scope, incumbent_context, operation);
    }
    let microtasks = pin!(v8::MicrotasksScope::new(
        scope,
        v8::MicrotasksScopeType::RunMicrotasks,
    ));
    with_incumbent_context(microtasks.init(), incumbent_context, operation)
}

fn with_incumbent_context<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    incumbent_context: v8::Local<'s, v8::Context>,
    operation: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let incumbent_scope = std::pin::pin!(v8::BackupIncumbentScope::new(incumbent_context));
    let _incumbent_scope = incumbent_scope.init();
    operation(scope)
}

/// Enters a prepared callback function's captured contexts and invokes it
/// through the host-supplied call boundary.
///
/// The callback-function type established `IsCallable` during Web IDL
/// conversion, so this path cannot represent callback-interface operation
/// lookup. Callable proxies are intentionally preserved as objects until this
/// final call boundary.
pub fn invoke_webidl_callback_function<'s, 'a, R, E>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &PreparedWebIdlCallbackFunction,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &'a [v8::Local<'s, v8::Value>],
    invoke_function: impl FnOnce(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Function>,
        v8::Local<'s, v8::Value>,
        &'a [v8::Local<'s, v8::Value>],
    ) -> Result<R, E>,
) -> Result<R, E> {
    let relevant_context = callback.relevant_context(scope);
    let incumbent_context = callback.incumbent_context(scope);
    with_webidl_callback_contexts(scope, relevant_context, incumbent_context, |scope| {
        let callback = callback.callback(scope);
        // SAFETY: `WebIdlCallbackFunction::try_new` is the only
        // constructor and records the callback only after V8 `IsCallable`
        // succeeds. Callable proxies remain callable objects even after
        // revocation; invoking a revoked proxy throws through the host
        // call boundary.
        let callback = unsafe { v8::Local::<v8::Function>::cast_unchecked(callback) };
        invoke_function(scope, callback, receiver, arguments)
    })
}

/// Resolves and invokes a Web IDL callback in the already-entered callback
/// contexts.
///
/// `invoke_function` owns the host's exception/reporting policy for the actual
/// call. `capture_resolution_failure` is invoked for a non-callable callback or
/// for callback-interface operation lookup failure while the exact `TryCatch`
/// state is still available.
pub fn invoke_webidl_callback<'s, 'a, R, E>(
    scope: &mut v8::PinScope<'s, '_>,
    invocation: WebIdlCallbackInvocation<'s, 'a>,
    invoke_function: impl FnOnce(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Function>,
        v8::Local<'s, v8::Value>,
        &'a [v8::Local<'s, v8::Value>],
    ) -> Result<R, E>,
    mut capture_resolution_failure: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        WebIdlCallbackResolutionFailure<'s, 'a>,
    ) -> E,
) -> Result<R, E> {
    let (callback, receiver) = if invocation.callable_at_conversion {
        if !invocation.callback.is_callable() {
            return Err(capture_not_callable(
                scope,
                invocation.callback,
                invocation.arguments,
                "The provided callback is not callable.",
                &mut capture_resolution_failure,
            ));
        }
        // Web IDL's callable branch includes callable proxies. A revoked proxy
        // still passes the conversion-time `IsCallable` fact and then throws
        // when V8 attempts the actual call.
        let callback = unsafe { v8::Local::<v8::Function>::cast_unchecked(invocation.callback) };
        (callback, invocation.callback_this)
    } else {
        let callback = resolve_callback_operation(
            scope,
            invocation.callback,
            invocation.operation_name,
            invocation.arguments,
            &mut capture_resolution_failure,
        )?;
        (v8::Local::new(scope, &callback), invocation.callback.into())
    };

    invoke_function(scope, callback, receiver, invocation.arguments)
}

fn resolve_callback_operation<'s, 'a, E>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_object: v8::Local<'s, v8::Object>,
    operation_name: &str,
    arguments: &'a [v8::Local<'s, v8::Value>],
    capture_failure: &mut impl FnMut(
        &mut v8::PinScope<'s, '_>,
        WebIdlCallbackResolutionFailure<'s, 'a>,
    ) -> E,
) -> Result<v8::Global<v8::Function>, E> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let operation_key = v8::String::new(&scope, operation_name)
        .expect("short callback operation name should allocate");
    let operation = callback_object.get(&scope, operation_key.into());
    let callable = operation
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .filter(|operation| operation.is_callable())
        .map(|operation| unsafe { v8::Local::<v8::Function>::cast_unchecked(operation) })
        .map(|operation| v8::Global::new(&scope, operation));

    if let Some(callable) = callable {
        return Ok(callable);
    }
    if scope.exception().is_none() {
        let message = v8::String::new(
            &scope,
            &format!("The provided callback has no callable {operation_name} property."),
        )
        .expect("short callback TypeError message should allocate");
        let exception = v8::Exception::type_error(&scope, message);
        scope.throw_exception(exception);
    }
    let failure = WebIdlCallbackResolutionFailure {
        callback: callback_object,
        arguments,
        exception: scope.exception(),
        message: scope.message(),
        stack_trace: scope.stack_trace(),
    };
    Err(capture_failure(&mut scope, failure))
}

fn capture_not_callable<'s, 'a, E>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_object: v8::Local<'s, v8::Object>,
    arguments: &'a [v8::Local<'s, v8::Value>],
    message: &str,
    capture_failure: &mut impl FnMut(
        &mut v8::PinScope<'s, '_>,
        WebIdlCallbackResolutionFailure<'s, 'a>,
    ) -> E,
) -> E {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let message =
        v8::String::new(&scope, message).expect("short callback TypeError should allocate");
    let exception = v8::Exception::type_error(&scope, message);
    scope.throw_exception(exception);
    let failure = WebIdlCallbackResolutionFailure {
        callback: callback_object,
        arguments,
        exception: scope.exception(),
        message: scope.message(),
        stack_trace: scope.stack_trace(),
    };
    capture_failure(&mut scope, failure)
}

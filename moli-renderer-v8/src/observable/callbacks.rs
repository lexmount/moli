use crate::{
    callback_invocation::invoke_synchronous_webidl_callback_function,
    exception_reporting::build_exception_report_without_stack,
    util::context_host_ptr_from_global_bridge,
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction, webidl,
};

pub(super) fn trace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: webidl::WebIdlCallbackFunction,
) -> v8::Local<'s, v8::Object> {
    V8TracedWebIdlCallbackFunction::new(scope, callback).into_object()
}

fn context_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> bool {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        host.window_execution_context_identity_for_v8_context(scope, context)
            .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
    } else {
        crate::worker::get_worker_state(scope).is_some()
    }
}

pub(super) fn is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    object
        .get_creation_context(scope)
        .is_some_and(|context| context_is_current(scope, context))
}

/// The initializer rethrows into Subscriber.error; observer and teardown
/// callbacks report errors in their own relevant realm. Neither path runs an
/// extra microtask checkpoint.
pub(super) fn invoke<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    carrier: v8::Local<'s, v8::Object>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    let context = callback.relevant_context(scope);
    if !context_is_current(scope, context) {
        return None;
    }
    v8::tc_scope!(let scope, scope);
    let receiver = v8::undefined(scope).into();
    invoke_synchronous_webidl_callback_function(scope, &callback, receiver, arguments);
    let exception = scope.exception();
    scope.reset();
    exception
}

pub(super) fn report_callback_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    carrier: v8::Local<'s, v8::Object>,
    exception: v8::Local<'s, v8::Value>,
) {
    let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    let context = callback.relevant_context(scope);
    report_in_context(scope, context, exception);
}

pub(super) fn report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
    exception: v8::Local<'s, v8::Value>,
) {
    if let Some(context) = subscriber.get_creation_context(scope) {
        report_in_context(scope, context, exception);
    }
}

pub(super) fn report_default_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
) {
    // An internal observer's default error algorithm reports in the current
    // realm. An error pushed after closure instead uses Subscriber's realm.
    let context = scope.get_current_context();
    report_in_context(scope, context, exception);
}

fn report_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    exception: v8::Local<'s, v8::Value>,
) {
    if !context_is_current(scope, context) {
        return;
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let message = v8::Exception::create_message(scope, exception);
    let report = build_exception_report_without_stack(scope, Some(exception), Some(message));
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let identity =
            unsafe { &*host_ptr }.window_execution_context_identity_for_v8_context(scope, context);
        crate::host::report_event_callback_exception(
            scope,
            host_ptr,
            "Observable",
            identity,
            None,
            &report,
        );
    } else {
        crate::worker::dispatch_current_worker_callback_exception(scope, report);
    }
}

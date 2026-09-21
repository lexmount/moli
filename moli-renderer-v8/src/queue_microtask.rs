//! Typed Web IDL callback residence for `queueMicrotask(VoidFunction)`.
//!
//! V8 remains the sole microtask queue and checkpoint owner. This module
//! converts the page-supplied callback once, stores it in the data of one
//! browser-created microtask trampoline, and invokes it through the shared
//! relevant/incumbent-context boundary. It does not create a Page task,
//! timer, owner wake, explicit checkpoint, or retry path.

use moli_webidl_callback::invoke_webidl_callback_function;

use crate::{
    context_bootstrap::require_same_origin_window_receiver,
    exception_reporting::{CallbackExceptionLogLevel, invoke_callback_with_report},
    host::report_event_callback_exception,
    util::context_host_ptr_from_global_bridge,
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    webidl,
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunctionOutcome, V8TracedWindowWebIdlCallbackFunction,
    },
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "queueMicrotask")]
struct QueueMicrotaskArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to execute 'queueMicrotask': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

pub(crate) fn window_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<QueueMicrotaskArgs>(scope, &args) else {
        return;
    };
    let callback = V8TracedWindowWebIdlCallbackFunction::new(scope, parsed.callback).into_object();
    let trampoline = v8::Function::builder(run_window_queue_microtask_callback)
        .data(callback.into())
        .build(scope)
        .expect("Window queueMicrotask trampoline should allocate");
    scope.enqueue_microtask(trampoline);
    rv.set_undefined();
}

fn run_window_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let carrier = v8::Local::<v8::Object>::try_from(args.data())
        .expect("Window queueMicrotask trampoline must retain its callback carrier");
    let host_ptr = context_host_ptr_from_global_bridge(scope)
        .expect("Window queueMicrotask trampoline must retain its host bridge");
    let host = unsafe { &*host_ptr };
    let callback = V8TracedWindowWebIdlCallbackFunction::from_object(carrier).prepare(scope, host);
    let relevant_identity = callback.relevant_identity();
    let receiver = v8::undefined(scope);
    match callback.invoke(
        scope,
        host,
        receiver.into(),
        &[],
        |scope, callback, receiver, arguments| {
            invoke_callback_with_report(
                scope,
                "callback",
                "queueMicrotask callback threw",
                CallbackExceptionLogLevel::Debug,
                "queueMicrotask callback",
                callback,
                receiver,
                arguments,
            )
        },
    ) {
        PreparedWindowWebIdlCallbackFunctionOutcome::Returned(_) => {}
        PreparedWindowWebIdlCallbackFunctionOutcome::Failed(report) => {
            report_event_callback_exception(
                scope,
                host_ptr,
                "queueMicrotask",
                relevant_identity,
                None,
                &report,
            );
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {}
    }
    rv.set_undefined();
}

pub(crate) fn worker_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<QueueMicrotaskArgs>(scope, &args) else {
        return;
    };
    let callback = V8TracedWebIdlCallbackFunction::new(scope, parsed.callback).into_object();
    let trampoline = v8::Function::builder(run_worker_queue_microtask_callback)
        .data(callback.into())
        .build(scope)
        .expect("worker queueMicrotask trampoline should allocate");
    scope.enqueue_microtask(trampoline);
    rv.set_undefined();
}

fn run_worker_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::worker::get_worker_state(scope).is_none() {
        rv.set_undefined();
        return;
    }
    let carrier = v8::Local::<v8::Object>::try_from(args.data())
        .expect("worker queueMicrotask trampoline must retain its callback carrier");
    let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    let receiver = v8::undefined(scope);
    if let Err(report) = invoke_webidl_callback_function(
        scope,
        &callback,
        receiver.into(),
        &[],
        |scope, callback, receiver, arguments| {
            invoke_callback_with_report(
                scope,
                "callback",
                "worker queueMicrotask callback threw",
                CallbackExceptionLogLevel::Debug,
                "worker queueMicrotask callback",
                callback,
                receiver,
                arguments,
            )
        },
    ) {
        let _ = crate::worker::dispatch_current_worker_callback_exception(scope, *report);
    }
    rv.set_undefined();
}

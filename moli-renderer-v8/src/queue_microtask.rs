//! Typed Web IDL callback residence for `queueMicrotask(VoidFunction)`.
//!
//! V8 remains the sole microtask queue and checkpoint owner. This module
//! converts the page-supplied callback once, stores it in the data of one
//! browser-created microtask trampoline, and invokes it through the shared
//! relevant/incumbent-context boundary. It does not create a Page task,
//! timer, owner wake, explicit checkpoint, or retry path.

use crate::{
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction, webidl,
    webidl_callback_execution::PreparedWebIdlCallbackExecution,
};

// The two global surfaces enqueue the same browser-created V8 trampoline.
pub(crate) fn window_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !crate::context_bootstrap::require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    queue_microtask_callback(scope, args, rv);
}
pub(crate) use queue_microtask_callback as worker_queue_microtask_callback;

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

pub(crate) fn queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<QueueMicrotaskArgs>(scope, &args) else {
        return;
    };
    let callback = V8TracedWebIdlCallbackFunction::new(scope, parsed.callback).into_object();
    let trampoline = v8::Function::builder(run_queue_microtask_callback)
        .data(callback.into())
        .build(scope)
        .expect("queueMicrotask trampoline should allocate");
    scope.enqueue_microtask(trampoline);
    rv.set_undefined();
}

fn run_queue_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let carrier = v8::Local::<v8::Object>::try_from(args.data())
        .expect("queueMicrotask trampoline must retain its callback carrier");
    if let Some(callback) = PreparedWebIdlCallbackExecution::from_traced(scope, carrier) {
        let receiver = v8::undefined(scope);
        callback.invoke_and_report(
            scope,
            "queueMicrotask",
            "queueMicrotask callback",
            receiver.into(),
            &[],
        );
    }
    rv.set_undefined();
}

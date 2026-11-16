//! Synchronous Web API callback calls with relevant/incumbent realm entry.

use moli_webidl_callback::{PreparedWebIdlCallbackFunction, invoke_webidl_callback_function};

/// Invokes one synchronous Web IDL callback function while preserving a thrown
/// V8 exception for the calling Web API algorithm.
///
/// This helper does not report exceptions, perform a microtask checkpoint, or
/// acquire Page/Document authority. The synchronous API owner remains
/// responsible for receiver/argument construction, abrupt completion, and
/// result conversion.
pub fn invoke_synchronous_webidl_callback_function<'s>(
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

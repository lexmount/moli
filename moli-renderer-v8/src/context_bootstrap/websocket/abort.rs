use super::helpers::websocket_id;
use super::stream::reject_websocket_stream_abort;
use super::*;
use crate::abort_signal_route::ResolvedAbortSignal;

/// The pending connection owns its AbortSignal registration.
/// Callback data contains the native connection ID.
pub(crate) struct WebSocketStreamAbortRegistration {
    signal: v8::Global<v8::Object>,
    algorithm: v8::Weak<v8::Function>,
}

impl WebSocketStreamAbortRegistration {
    fn remove(self, scope: &mut v8::PinScope<'_, '_>) {
        let signal = v8::Local::new(scope, &self.signal);
        if let Some(algorithm) = self.algorithm.to_local(scope)
            && let Some(signal) = ResolvedAbortSignal::resolve(scope, signal)
        {
            signal.unregister_algorithm(scope, algorithm);
        }
    }
}

pub(super) fn install_websocket_stream_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket_id: u64,
    signal: ResolvedAbortSignal<'s>,
) {
    let Some(algorithm) = v8::Function::builder(abort_callback)
        .data(v8::Number::new(scope, socket_id as f64).into())
        .build(scope)
    else {
        return;
    };
    if !signal.register_algorithm(scope, algorithm) {
        return;
    }
    let registration = WebSocketStreamAbortRegistration {
        signal: v8::Global::new(scope, signal.value()),
        algorithm: v8::Weak::new(scope, algorithm),
    };
    let retained = context_host_ptr_from_global_bridge(scope).is_some_and(|host| {
        unsafe { &mut *host }.set_websocket_stream_abort_registration(socket_id, registration)
    });
    if !retained {
        signal.unregister_algorithm(scope, algorithm);
    }
}

pub(super) fn remove_websocket_stream_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(socket_id) = websocket_id(scope, stream) else {
        return;
    };
    let registration = context_host_ptr_from_global_bridge(scope)
        .and_then(|host| unsafe { &mut *host }.take_websocket_stream_abort_registration(socket_id));
    if let Some(registration) = registration {
        registration.remove(scope);
    }
}

fn abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
    let Some(socket_id) = args.data().number_value(scope).map(|id| id as u64) else {
        return;
    };
    let Some(host) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // A retained signal must never revive a retired connection or enter its
    // replacement realm. The existing transport owner check also retires stale ids.
    let Some((_, _, _, stream)) = unsafe { &mut *host }.websocket_dispatch_target(scope, socket_id)
    else {
        return;
    };
    remove_websocket_stream_abort(scope, stream);
    let _ = unsafe { &mut *host }.close_websocket(socket_id, None, String::new());
    reject_websocket_stream_abort(scope, stream);
}

use super::helpers::websocket_id;
use super::stream::reject_websocket_stream_abort;
use super::transport::{close_websocket_stream_transport, websocket_stream_transport_wrapper};
use super::*;
use crate::abort_signal_route::ResolvedAbortSignal;

/// The connection owns these roots only while its handshake can be aborted.
/// The signal stores a weak algorithm, whose callback data is just a socket id.
/// Retiring the connection therefore cannot keep its wrapper alive via signal.
pub(crate) struct WebSocketStreamAbortRegistration {
    signal: v8::Global<v8::Object>,
    algorithm: v8::Global<v8::Function>,
}

impl WebSocketStreamAbortRegistration {
    #[cfg(test)]
    pub(crate) fn weak_algorithm_for_test(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> v8::Weak<v8::Function> {
        let algorithm = v8::Local::new(scope, &self.algorithm);
        v8::Weak::new(scope, algorithm)
    }

    fn remove(self, scope: &mut v8::PinScope<'_, '_>) {
        let signal = v8::Local::new(scope, &self.signal);
        let algorithm = v8::Local::new(scope, &self.algorithm);
        if let Some(signal) = ResolvedAbortSignal::resolve(scope, signal) {
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
    if !signal.register_weak_algorithm(scope, algorithm) {
        return;
    }
    let registration = WebSocketStreamAbortRegistration {
        signal: v8::Global::new(scope, signal.value()),
        algorithm: v8::Global::new(scope, algorithm),
    };
    let retained = if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.set_websocket_stream_abort_registration(socket_id, registration)
    } else {
        crate::worker::set_worker_websocket_stream_abort_registration(
            scope,
            socket_id,
            registration,
        )
    };
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
    let registration = if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.take_websocket_stream_abort_registration(socket_id)
    } else {
        crate::worker::take_worker_websocket_stream_abort_registration(scope, socket_id)
    };
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
    // A retained signal must never revive a retired connection or enter its
    // replacement realm. The existing transport owner check also retires stale ids.
    let Some(stream) = websocket_stream_transport_wrapper(scope, socket_id) else {
        return;
    };
    remove_websocket_stream_abort(scope, stream);
    close_websocket_stream_transport(scope, socket_id, None, String::new());
    reject_websocket_stream_abort(scope, stream);
}

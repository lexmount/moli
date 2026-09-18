use crate::abort_signal_route::AbortAlgorithm;

pub(super) fn invoke_abort_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'_, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    abort_algorithms: Vec<AbortAlgorithm>,
) -> bool {
    let signal = local_object_in_scope(scope, signal);
    crate::abort_signal_route::invoke_abort_algorithms(
        scope,
        "AbortSignal abort algorithm",
        signal,
        reason,
        abort_algorithms,
    )
}

pub(super) fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

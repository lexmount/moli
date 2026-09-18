use crate::abort_signal_route::{AbortAlgorithm, invoke_abort_algorithm};

pub(super) fn invoke_abort_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'_, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    abort_algorithms: Vec<AbortAlgorithm>,
) -> bool {
    let signal = local_object_in_scope(scope, signal);
    for algorithm in abort_algorithms {
        let Some(algorithm) = algorithm.prepare(scope) else {
            continue;
        };
        if !invoke_abort_algorithm(
            scope,
            "AbortSignal abort algorithm",
            algorithm,
            signal,
            reason,
        ) {
            return false;
        }
    }
    true
}

pub(super) fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

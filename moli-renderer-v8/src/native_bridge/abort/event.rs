use crate::exception_reporting::invoke_callback;

pub(super) fn invoke_abort_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'_, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    abort_algorithms: Vec<v8::Global<v8::Function>>,
) {
    let signal = local_object_in_scope(scope, signal);
    for algorithm in abort_algorithms {
        let algorithm = v8::Local::new(scope, &algorithm);
        let _ = invoke_callback(
            scope,
            "AbortSignal abort algorithm",
            algorithm,
            signal.into(),
            &[reason],
        );
    }
}

pub(super) fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

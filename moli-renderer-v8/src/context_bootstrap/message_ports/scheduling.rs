use crate::util::enqueue_host_microtask;

pub(in crate::context_bootstrap) fn schedule_scope_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: Option<v8::Local<'s, v8::Value>>,
) {
    let mut builder = v8::Function::builder(callback);
    if let Some(data) = data {
        builder = builder.data(data);
    }
    let Some(callback) = builder.build(scope) else {
        return;
    };
    enqueue_host_microtask(scope, callback);
}

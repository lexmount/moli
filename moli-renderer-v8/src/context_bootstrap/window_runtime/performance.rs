use crate::context_bootstrap::performance_runtime::performance_now;
use moli_webapi_declare::v8;

pub(in crate::context_bootstrap) fn performance_now_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let now = performance_now(scope, args.this());
    rv.set(v8::Number::new(scope, now).into());
}

use crate::context_bootstrap::{
    PERFORMANCE_TIME_ORIGIN_SLOT, dom_time_since_origin_millis, performance_slot_number,
};
use moli_webapi_declare::v8;

pub(in crate::context_bootstrap) fn performance_now_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let time_origin = performance_slot_number(scope, args.this(), PERFORMANCE_TIME_ORIGIN_SLOT)
        .expect("branded Performance receiver should retain its time origin");
    let now = dom_time_since_origin_millis(time_origin);
    rv.set(v8::Number::new(scope, now).into());
}

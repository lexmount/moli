use super::*;

pub(super) fn bridge_handle_lookup_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    lookup: impl FnOnce(&PageScriptHost, DomHandle) -> Option<DomHandle>,
) {
    let Some(handle) = callback_arg_dom_handle(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let result = lookup(unsafe { &*runtime_ptr }, handle);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, result);
}

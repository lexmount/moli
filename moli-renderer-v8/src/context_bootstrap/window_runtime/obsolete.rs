use crate::context_bootstrap::require_same_origin_window_receiver;

pub(in crate::context_bootstrap) fn window_obsolete_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    rv.set_undefined();
}

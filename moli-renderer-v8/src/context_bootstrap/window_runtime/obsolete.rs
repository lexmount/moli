pub(in crate::context_bootstrap) fn window_obsolete_noop_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_undefined();
}

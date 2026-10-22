pub(crate) fn document_fullscreen_getter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_bool(false);
}

pub(crate) fn fullscreen_element_getter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_null();
}

pub(crate) fn fullscreen_lenient_setter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
}

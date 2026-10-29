use super::event_backing;

pub(in crate::context_bootstrap) fn ui_event_which_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // The declaration's generated receiver check requires a native UIEvent.
    // Older native UI event producers do not store the legacy which member.
    let state = event_backing(scope, args.this());
    let Some(value) = state.get(scope, args.data()) else {
        return;
    };
    if value.is_undefined() {
        rv.set_uint32(0);
    } else {
        rv.set(value);
    }
}

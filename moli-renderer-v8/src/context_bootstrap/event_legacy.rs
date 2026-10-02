use crate::util::context_host_ptr_from_window_object;
use moli_webapi::events::legacy::initialize_legacy_text_event;

pub(super) use moli_webapi::events::legacy::{
    event_init_event_callback, mouse_event_init_callback,
};

pub(super) fn text_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    initialize_legacy_text_event(scope, args, |scope, window| {
        context_host_ptr_from_window_object(scope, window).is_some()
    });
}

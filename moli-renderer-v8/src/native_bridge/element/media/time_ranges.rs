use super::super::html_media_element_getter_receiver;

pub(in crate::native_bridge) fn media_buffered_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    media_ranges_getter(scope, args.this(), "buffered", rv);
}

pub(in crate::native_bridge) fn media_played_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    media_ranges_getter(scope, args.this(), "played", rv);
}

pub(in crate::native_bridge) fn media_seekable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    media_ranges_getter(scope, args.this(), "seekable", rv);
}

fn media_ranges_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if html_media_element_getter_receiver(scope, receiver, member).is_none() {
        return;
    }
    // The current media backend does not decode/buffer media or advance a
    // playback timeline. Do not invent loaded or played ranges from currentTime.
    // Each read still returns an independent native snapshot.
    rv.set(crate::context_bootstrap::new_time_ranges_value(scope, &[]).into());
}

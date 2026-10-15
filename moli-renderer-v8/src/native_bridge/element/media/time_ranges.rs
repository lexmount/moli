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
    let Some((runtime_ptr, handle)) = html_media_element_getter_receiver(scope, receiver, member)
    else {
        return;
    };
    if member == "seekable"
        && let Some(duration) = unsafe { &*runtime_ptr }.media_duration(handle)
        && duration.is_finite()
        && duration > 0.0
    {
        rv.set(crate::context_bootstrap::new_time_ranges_value(scope, &[(0.0, duration)]).into());
        return;
    }
    // Played intervals and buffering require their own resource state;
    // currentTime alone does not establish either range.
    rv.set(crate::context_bootstrap::new_time_ranges_value(scope, &[]).into());
}

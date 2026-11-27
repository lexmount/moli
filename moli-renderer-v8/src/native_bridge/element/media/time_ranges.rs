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
    let Some((_host, _handle)) = html_media_element_getter_receiver(scope, receiver, member) else {
        return;
    };
    let Some(receiver) = moli_webapi_declare::web_api_object_target(scope, receiver) else {
        return;
    };
    let Some(context) = receiver.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let ranges: &[(f64, f64)] = &[];
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    let video_ranges;
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    let ranges = if member == "buffered" || member == "seekable" {
        video_ranges = unsafe { &*_host }.video_buffered(_handle);
        video_ranges.as_slice()
    } else {
        ranges
    };
    rv.set(crate::context_bootstrap::new_time_ranges_value(scope, ranges).into());
}

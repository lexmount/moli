use crate::util::v8_string;
use crate::webidl;
use moli_web_mime::media_mime_support;

use super::super::{
    apply_default_text_track_modes_for_media, construct_simple_event, dispatch_public_event,
    dispatch_text_control_event, html_media_element_method_receiver,
    refresh_media_active_text_track_cues,
};
use super::attributes::queue_media_load_for_explicit_request;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLMediaElement.canPlayType")]
struct MediaCanPlayTypeArgs {
    #[webidl(required)]
    media_type: String,
}

fn media_can_play_type_result(input: &str) -> &'static str {
    media_mime_support(input).as_can_play_type()
}

pub(in crate::native_bridge) fn media_play_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_method_receiver(scope, args.this(), "play")
    else {
        rv.set_undefined();
        return;
    };
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    {
        if unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| element.is_html_element("video"))
        {
            queue_media_load_for_explicit_request(scope, runtime_ptr, handle);
        }
        if unsafe { &*runtime_ptr }.manages_video(handle) {
            if let Some(promise) = play_decoded_video(scope, runtime_ptr, handle) {
                rv.set(promise.into());
            }
            return;
        }
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_media_paused(handle, false);
    let _ = runtime.set_media_ready_state(handle, 4);
    let _ = runtime.set_media_network_state(handle, 1);
    apply_default_text_track_modes_for_media(scope, runtime_ptr, handle);
    refresh_media_active_text_track_cues(scope, runtime_ptr, handle);
    if changed {
        dispatch_text_control_event(scope, runtime_ptr, handle, "play");
        dispatch_text_control_event(scope, runtime_ptr, handle, "playing");
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let undefined = v8::undefined(scope);
    let _ = resolver.resolve(scope, undefined.into());
    rv.set(promise.into());
}

pub(in crate::native_bridge) fn media_pause_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_method_receiver(scope, args.this(), "pause")
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_media_paused(handle, true);
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    runtime.stop_video_clock(scope, handle);
    if changed {
        dispatch_text_control_event(scope, runtime_ptr, handle, "pause");
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_load_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_method_receiver(scope, args.this(), "load")
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    runtime.reset_video_playback(scope, handle);
    let _ = runtime.cancel_pending_media_load_sequence(handle);
    let error_changed = runtime.set_media_error_code(handle, None);
    let paused_changed = runtime.set_media_paused(handle, true);
    let time_changed = runtime.set_media_current_time(handle, 0.0);
    let seeking_changed = runtime.set_media_seeking(handle, false);
    let ready_changed = runtime.set_media_ready_state(handle, 0);
    let network_changed = runtime.set_media_network_state(handle, 0);
    if (error_changed
        || paused_changed
        || time_changed
        || seeking_changed
        || ready_changed
        || network_changed)
        && let Some(event) = construct_simple_event(scope, "emptied", false, false, false)
    {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    queue_media_load_for_explicit_request(scope, runtime_ptr, handle);
    rv.set_undefined();
}

#[cfg(all(
    feature = "media-ext",
    any(target_os = "linux", target_os = "macos"),
    target_pointer_width = "64"
))]
fn play_decoded_video<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::PageScriptHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Promise>> {
    use crate::dom::native::{Element, Node};

    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let host = unsafe { &mut *host_ptr };
    if host
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::media_error_code)
        .is_some()
    {
        let error = crate::native_bridge::abort::dom_exception_value(
            scope,
            "The video source could not be played.",
            "NotSupportedError",
        );
        let _ = resolver.reject(scope, error);
        return Some(promise);
    }
    let changed = host.set_media_paused(handle, false);
    host.wait_for_video_play(scope, handle, resolver);
    if host.video_ended(handle)
        && let Some(token) = host.dom_host_mut().advance_media_seek_token(handle)
    {
        host.set_media_seeking(handle, true);
        let _ = host.queue_media_seeking_event(scope, handle);
        host.seek_video(scope, handle, 0.0, token);
    }
    apply_default_text_track_modes_for_media(scope, host_ptr, handle);
    refresh_media_active_text_track_cues(scope, host_ptr, handle);
    if changed {
        dispatch_text_control_event(scope, host_ptr, handle, "play");
    }
    crate::native_bridge::start_video_if_playing(scope, host_ptr, handle);
    Some(promise)
}

pub(in crate::native_bridge) fn media_can_play_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if html_media_element_method_receiver(scope, args.this(), "canPlayType").is_none() {
        rv.set_empty_string();
        return;
    }
    let Some(parsed) = webidl::parse_args::<MediaCanPlayTypeArgs>(scope, &args) else {
        return;
    };
    if let Some(result) = v8_string(scope, media_can_play_type_result(&parsed.media_type)) {
        rv.set(result.into());
    } else {
        rv.set_empty_string();
    }
}

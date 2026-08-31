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

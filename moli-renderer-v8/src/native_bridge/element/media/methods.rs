use crate::util::v8_string;
use crate::webidl;
use moli_web_mime::media_mime_support;

use super::super::{
    apply_default_text_track_modes_for_media, html_media_element_method_receiver,
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
    let Some(realm) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let _ = unsafe { &mut *runtime_ptr }.bind_media_playback_global(scope, handle, args.this());
    crate::native_bridge::JsContextHost::request_media_play(scope, runtime_ptr, handle, resolver);
    apply_default_text_track_modes_for_media(scope, runtime_ptr, handle);
    refresh_media_active_text_track_cues(scope, runtime_ptr, handle);
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
    let _ = unsafe { &mut *runtime_ptr }.bind_media_playback_global(scope, handle, args.this());
    let empty = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| element.media_network_state() == 0);
    if empty {
        queue_media_load_for_explicit_request(scope, runtime_ptr, handle);
    }
    unsafe { &mut *runtime_ptr }.pause_media_playback(scope, handle);
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
    let _ = runtime.bind_media_playback_global(scope, handle, args.this());
    runtime.reset_media_playback_for_load(scope, handle);
    let _ = runtime.cancel_pending_media_load_sequence(handle);
    let error_changed = runtime.set_media_error_code(handle, None);
    let paused_changed = runtime.set_media_paused(handle, true);
    let time_changed = runtime.set_media_current_time(handle, 0.0);
    let seeking_changed = runtime.set_media_seeking(handle, false);
    let ready_changed = runtime.set_media_ready_state(handle, 0);
    let network_changed = runtime.set_media_network_state(handle, 0);
    if error_changed
        || paused_changed
        || time_changed
        || seeking_changed
        || ready_changed
        || network_changed
    {
        runtime.queue_media_emptied(scope, handle);
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

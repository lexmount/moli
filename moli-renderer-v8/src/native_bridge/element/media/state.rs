use crate::document_runtime::DomHandle;
use crate::dom::native::{Element, Node};
use crate::util::{get_private_value, set_private_value};
use crate::webidl;

const MEDIA_ERROR_OBJECT_SLOT: &str = "__moliMediaElementErrorObject";
const MEDIA_ERROR_OBJECT_CODE_SLOT: &str = "__moliMediaElementErrorObjectCode";

use super::super::{
    construct_simple_event, dispatch_public_event, dispatch_text_control_event,
    html_media_element_getter_receiver, html_media_element_setter_receiver,
    refresh_media_active_text_track_cues,
};

pub(in crate::native_bridge) fn media_paused_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "paused")
    else {
        rv.set_bool(true);
        return;
    };
    let paused = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::media_paused);
    rv.set_bool(paused);
}

pub(in crate::native_bridge) fn media_error_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "error")
    else {
        rv.set_null();
        return;
    };
    let error_code = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::media_error_code);
    let Some(error_code) = error_code else {
        set_private_value(
            scope,
            args.this(),
            MEDIA_ERROR_OBJECT_SLOT,
            v8::undefined(scope).into(),
        );
        set_private_value(
            scope,
            args.this(),
            MEDIA_ERROR_OBJECT_CODE_SLOT,
            v8::undefined(scope).into(),
        );
        rv.set_null();
        return;
    };
    let cached_code = get_private_value(scope, args.this(), MEDIA_ERROR_OBJECT_CODE_SLOT)
        .and_then(|value| value.uint32_value(scope));
    if cached_code == Some(error_code)
        && let Some(error) = get_private_value(scope, args.this(), MEDIA_ERROR_OBJECT_SLOT)
        && !error.is_undefined()
    {
        rv.set(error);
        return;
    }
    let message = match error_code {
        1 => "The fetching process for the media resource was aborted by the user agent.",
        2 => "A network error prevented the media resource from being fetched.",
        3 => "An error occurred while decoding the media resource.",
        _ => "The media resource could not be loaded because its format is not supported.",
    };
    let Some(error) = crate::context_bootstrap::new_media_error_value(scope, error_code, message)
    else {
        rv.set_null();
        return;
    };
    set_private_value(scope, args.this(), MEDIA_ERROR_OBJECT_SLOT, error.into());
    set_private_value(
        scope,
        args.this(),
        MEDIA_ERROR_OBJECT_CODE_SLOT,
        v8::Integer::new_from_unsigned(scope, error_code).into(),
    );
    rv.set(error.into());
}

pub(in crate::native_bridge) fn media_volume_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "volume")
    else {
        rv.set(v8::Number::new(scope, 1.0).into());
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::media_volume)
        .unwrap_or(1.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::native_bridge) fn media_volume_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "volume")
    else {
        rv.set_undefined();
        return;
    };
    let Some(number) = media_double_value(scope, args.get(0), "volume") else {
        return;
    };
    if !(0.0..=1.0).contains(&number) {
        crate::native_bridge::bridge::throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "The volume must be in the range 0.0 to 1.0.",
        );
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let previous = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::media_volume);
    if runtime.set_media_volume(handle, number) && previous != Some(number) {
        runtime.queue_media_volume_change(scope, handle, args.this());
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_muted_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "muted")
    else {
        rv.set_bool(false);
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::media_muted);
    rv.set_bool(value);
}

pub(in crate::native_bridge) fn media_muted_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "muted")
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let muted = args.get(0).boolean_value(scope);
    if runtime.set_media_muted(handle, muted) {
        runtime.queue_media_volume_change(scope, handle, args.this());
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_playback_rate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "playbackRate")
    else {
        rv.set(v8::Number::new(scope, 1.0).into());
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::media_playback_rate)
        .unwrap_or(1.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::native_bridge) fn media_playback_rate_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "playbackRate")
    else {
        rv.set_undefined();
        return;
    };
    let Some(number) = media_double_value(scope, args.get(0), "playbackRate") else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.set_media_playback_rate(handle, number);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_current_time_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "currentTime")
    else {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| {
            let start = element.media_default_start_position();
            if start != 0.0 {
                start
            } else {
                element.media_current_time()
            }
        })
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::native_bridge) fn media_current_time_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_setter_receiver(scope, args.this(), "currentTime")
    else {
        rv.set_undefined();
        return;
    };
    let Some(number) = media_double_value(scope, args.get(0), "currentTime") else {
        return;
    };
    if unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.media_ready_state() == 0)
    {
        unsafe { &mut *runtime_ptr }.set_media_default_start_position(handle, number);
    } else {
        seek_media_to(scope, runtime_ptr, handle, number);
    }
    rv.set_undefined();
}

pub(crate) fn seek_media_to(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
    position: f64,
) {
    {
        let runtime = unsafe { &mut *runtime_ptr };
        let duration = runtime.media_duration(handle).unwrap_or(f64::INFINITY);
        runtime.set_media_seeking(handle, true);
        runtime.set_media_current_time(handle, position.max(0.0).min(duration));
    }
    schedule_media_seek_completion(scope, runtime_ptr, handle);
    refresh_media_active_text_track_cues(scope, runtime_ptr, handle);
}

fn schedule_media_seek_completion(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
) {
    let Some(token) = (unsafe { &mut *runtime_ptr })
        .dom_host_mut()
        .advance_media_seek_token(handle)
    else {
        return;
    };
    if !unsafe { &mut *runtime_ptr }.queue_media_seeking_event(scope, handle) {
        let _ = unsafe { &mut *runtime_ptr }.set_media_seeking(handle, false);
        return;
    }
    if unsafe { &mut *runtime_ptr }.defer_video_seek_completion(handle, token) {
        return;
    }
    if unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.media_ready_state() < 2)
    {
        return;
    }
    if !unsafe { &mut *runtime_ptr }.queue_media_seek_completion(scope, handle, token) {
        let _ = unsafe { &mut *runtime_ptr }.set_media_seeking(handle, false);
    }
}

pub(in crate::native_bridge) fn finish_media_seek_after_loaded_data(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
) {
    let seeking = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::media_seeking);
    if !seeking {
        return;
    }
    if unsafe { &mut *runtime_ptr }.finish_video_seek_if_ready(scope, handle) {
        return;
    }
    let Some(token) = unsafe { &*runtime_ptr }.dom_host().media_seek_token(handle) else {
        return;
    };
    if !unsafe { &mut *runtime_ptr }.defer_video_seek_completion(handle, token) {
        let _ = unsafe { &mut *runtime_ptr }.queue_media_seek_completion(scope, handle, token);
    }
}

pub(crate) fn dispatch_media_seeking_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> bool {
    if !media_element_is_current_event_target(unsafe { &*runtime_ptr }, handle) {
        return false;
    }
    fire_media_seek_event(scope, runtime_ptr, handle, "seeking");
    true
}

pub(crate) fn dispatch_media_seek_completion(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
    token: u64,
) -> bool {
    if !media_element_is_current_event_target(unsafe { &*runtime_ptr }, handle)
        || unsafe { &*runtime_ptr }.dom_host().media_seek_token(handle) != Some(token)
        || !unsafe { &mut *runtime_ptr }.set_media_seeking(handle, false)
    {
        return false;
    }
    // The completed seek owns its notification even if timeupdate starts a
    // new seek. Resource replacement still retires it through the load epoch.
    unsafe { &mut *runtime_ptr }.queue_media_seeked(scope, handle);
    crate::native_bridge::JsContextHost::process_media_playback_boundary(
        scope,
        runtime_ptr,
        handle,
    );
    fire_media_seek_event(scope, runtime_ptr, handle, "timeupdate");
    if unsafe { &*runtime_ptr }.dom_host().media_seek_token(handle) == Some(token) {
        unsafe { &mut *runtime_ptr }.notify_media_playing_after_seek(scope, handle);
    }
    true
}

fn fire_media_seek_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
    event_type: &str,
) {
    if let Some(event) = construct_simple_event(scope, event_type, false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
}

fn media_element_is_current_event_target(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::is_html_media)
}

fn media_double_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) -> Option<f64> {
    match webidl::convert::<webidl::Double>(
        scope,
        value,
        webidl::Context::member("HTMLMediaElement", property),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(in crate::native_bridge) fn media_duration_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "duration")
    else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let duration = unsafe { &*runtime_ptr }
        .media_duration(handle)
        .unwrap_or(f64::NAN);
    rv.set(v8::Number::new(scope, duration).into());
}

pub(in crate::native_bridge) fn media_ended_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "ended")
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(unsafe { &*runtime_ptr }.media_ended(handle));
}

pub(in crate::native_bridge) fn media_seeking_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "seeking")
    else {
        rv.set_bool(false);
        return;
    };
    let seeking = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(Element::media_seeking);
    rv.set_bool(seeking);
}

pub(in crate::native_bridge) fn media_ready_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "readyState")
    else {
        rv.set_uint32(0);
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::media_ready_state)
        .unwrap_or(0);
    rv.set_uint32(value);
}

pub(in crate::native_bridge) fn media_network_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_media_element_getter_receiver(scope, args.this(), "networkState")
    else {
        rv.set_uint32(0);
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::media_network_state)
        .unwrap_or(0);
    rv.set_uint32(value);
}

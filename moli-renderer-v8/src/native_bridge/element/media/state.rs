use crate::document_runtime::DomHandle;
use crate::dom::native::{Element, Node};
use crate::util::{get_private_value, set_private_value};
use crate::webidl;

const MEDIA_ERROR_OBJECT_SLOT: &str = "__moliMediaElementErrorObject";
const MEDIA_ERROR_OBJECT_CODE_SLOT: &str = "__moliMediaElementErrorObjectCode";

use super::super::{
    dispatch_text_control_event, html_media_element_getter_receiver,
    html_media_element_setter_receiver, refresh_media_active_text_track_cues,
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
    let Some(mut number) = media_double_value(scope, args.get(0), "volume") else {
        return;
    };
    number = number.clamp(0.0, 1.0);
    let _ = unsafe { &mut *runtime_ptr }.set_media_volume(handle, number);
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
    let _ = unsafe { &mut *runtime_ptr }.set_media_muted(handle, args.get(0).boolean_value(scope));
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
        .map(Element::media_current_time)
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
    let changed = {
        let runtime = unsafe { &mut *runtime_ptr };
        let time_changed = runtime.set_media_current_time(handle, number.max(0.0));
        let seeking_changed = runtime.set_media_seeking(handle, true);
        time_changed || seeking_changed
    };
    refresh_media_active_text_track_cues(scope, runtime_ptr, handle);
    if changed {
        schedule_media_seek_completion(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
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
    if !unsafe { &mut *runtime_ptr }.queue_media_seek_completion(scope, handle, token) {
        let _ = unsafe { &mut *runtime_ptr }.set_media_seeking(handle, false);
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
    dispatch_text_control_event(scope, runtime_ptr, handle, "seeking");
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
    dispatch_text_control_event(scope, runtime_ptr, handle, "seeked");
    true
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
    if html_media_element_getter_receiver(scope, args.this(), "duration").is_none() {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    }
    rv.set(v8::Number::new(scope, f64::NAN).into());
}

pub(in crate::native_bridge) fn media_ended_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_media_element_getter_receiver(scope, args.this(), "ended").is_none() {
        rv.set_bool(false);
        return;
    }
    rv.set_bool(false);
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

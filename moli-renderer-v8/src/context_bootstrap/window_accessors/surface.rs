use super::helpers::{
    window_child_context_handle, window_has_discarded_child_browsing_context, window_hidden_value,
    window_host_ptr, window_receiver,
};
use super::*;
use crate::{native_bridge::lightweight_popup_id_from_window, util::v8str, webidl};

fn window_inner_surface_dimension<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    width: bool,
) -> f64 {
    let fallback = if width {
        DEFAULT_WINDOW_SURFACE_PROFILE.inner_width
    } else {
        DEFAULT_WINDOW_SURFACE_PROFILE.inner_height
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        return fallback;
    };
    let host = unsafe { &*host_ptr };
    if let Some(frame_handle) = window_child_context_handle(scope, receiver) {
        return crate::native_bridge::element::iframe_handle_viewport(host, frame_handle)
            .and_then(|viewport| {
                if width {
                    viewport.width
                } else {
                    viewport.height
                }
            })
            .unwrap_or(fallback);
    }
    host.viewport_surface()
        .map(|viewport| {
            f64::from(if width {
                viewport.inner_width
            } else {
                viewport.inner_height
            })
        })
        .unwrap_or(fallback)
}

pub(in crate::context_bootstrap) fn window_inner_surface_width<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> f64 {
    window_inner_surface_dimension(scope, receiver, true)
}

pub(in crate::context_bootstrap) fn window_inner_surface_height<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> f64 {
    window_inner_surface_dimension(scope, receiver, false)
}

fn set_receiver_slot_or_undefined<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, args) else {
        return;
    };
    let value = if let Some(value) = window_hidden_value(scope, receiver, slot) {
        Some(value)
    } else {
        match super::super::window_lazy_surface::ensure_window_lazy_surface_value(
            scope, receiver, slot,
        ) {
            Ok(value) => value,
            Err(error) => {
                throw_error(
                    scope,
                    &format!("Failed to materialize Window surface: {error}"),
                );
                return;
            }
        }
    };
    match value {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn set_receiver_window_alias<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, args) else {
        return;
    };
    rv.set(
        window_hidden_value(scope, receiver, slot)
            .unwrap_or_else(|| scope.get_current_context().global(scope).into()),
    );
}

fn set_receiver_related_window_alias<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, args) else {
        return;
    };
    if window_has_discarded_child_browsing_context(scope, receiver) {
        rv.set_null();
        return;
    }
    rv.set(
        window_hidden_value(scope, receiver, slot)
            .unwrap_or_else(|| scope.get_current_context().global(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn window_opener_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    if window_has_discarded_child_browsing_context(scope, receiver) {
        rv.set_null();
        return;
    }
    if let Some(popup_id) = lightweight_popup_id_from_window(scope, receiver)
        && let Some(host_ptr) = window_host_ptr(scope, receiver)
        && let Some(opener) = unsafe { &*host_ptr }.lightweight_popup_opener_window(scope, popup_id)
    {
        rv.set(opener.into());
        return;
    }
    if let Some(handle) = window_child_context_handle(scope, receiver)
        && let Some(host_ptr) = window_host_ptr(scope, receiver)
        && let Some(opener) = unsafe { &*host_ptr }.child_browsing_context_opener(scope, handle)
    {
        rv.set(opener.into());
        return;
    }
    rv.set_null();
}

pub(in crate::context_bootstrap) fn window_closed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    if let Some(popup_id) = lightweight_popup_id_from_window(scope, receiver) {
        let closed = window_host_ptr(scope, receiver)
            .is_none_or(|host_ptr| !unsafe { &*host_ptr }.lightweight_popup_is_open(popup_id));
        rv.set_bool(closed);
        return;
    }
    rv.set_bool(window_has_discarded_child_browsing_context(scope, receiver));
}

pub(in crate::context_bootstrap) fn window_opener_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let value = args.get(0);
    if value.is_null() {
        if let Some(host_ptr) = window_host_ptr(scope, receiver) {
            let host = unsafe { &mut *host_ptr };
            if let Some(popup_id) = lightweight_popup_id_from_window(scope, receiver) {
                host.clear_lightweight_popup_opener(popup_id);
            } else if let Some(handle) = window_child_context_handle(scope, receiver) {
                host.clear_child_browsing_context_opener(handle);
            }
        }
        return;
    }
    match receiver.define_own_property(
        scope,
        v8str(scope, "opener").into(),
        value,
        v8::PropertyAttribute::NONE,
    ) {
        Some(true) => {}
        Some(false) => {
            webidl::throw_type_error(scope, "Failed to replace Window.opener property.");
        }
        None => {}
    }
}

pub(in crate::context_bootstrap) fn window_inner_width_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let width = window_inner_surface_width(scope, receiver);
    rv.set(v8::Number::new(scope, width).into());
}

pub(in crate::context_bootstrap) fn window_inner_height_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let height = window_inner_surface_height(scope, receiver);
    rv.set(v8::Number::new(scope, height).into());
}

pub(in crate::context_bootstrap) fn window_outer_width_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if window_receiver(scope, &args).is_some() {
        rv.set(v8::Number::new(scope, DEFAULT_WINDOW_SURFACE_PROFILE.inner_width).into());
    }
}

pub(in crate::context_bootstrap) fn window_outer_height_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if window_receiver(scope, &args).is_some() {
        rv.set(v8::Number::new(scope, DEFAULT_WINDOW_SURFACE_PROFILE.inner_height).into());
    }
}

pub(in crate::context_bootstrap) fn window_device_pixel_ratio_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if window_receiver(scope, &args).is_some() {
        rv.set(v8::Number::new(scope, DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio).into());
    }
}

pub(in crate::context_bootstrap) fn window_scroll_x_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let value = window_hidden_value(scope, receiver, WINDOW_SCROLL_X_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::context_bootstrap) fn window_scroll_y_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let value = window_hidden_value(scope, receiver, WINDOW_SCROLL_Y_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::context_bootstrap) fn window_navigator_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_NAVIGATOR_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_screen_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_SCREEN_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_speech_synthesis_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_SPEECH_SYNTHESIS_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_custom_elements_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_CUSTOM_ELEMENTS_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_performance_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_PERFORMANCE_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_visual_viewport_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_slot_or_undefined(scope, &args, WINDOW_VISUAL_VIEWPORT_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_window_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_window_alias(scope, &args, WINDOW_SELF_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_top_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_related_window_alias(scope, &args, WINDOW_TOP_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_parent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_related_window_alias(scope, &args, WINDOW_PARENT_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_frames_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_window_alias(scope, &args, WINDOW_FRAMES_SLOT, rv);
}

pub(in crate::context_bootstrap) fn window_self_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_receiver_window_alias(scope, &args, WINDOW_SELF_SLOT, rv);
}

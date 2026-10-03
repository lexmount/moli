use crate::util::v8_string;
use std::str::FromStr;

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::{
    attribute_property_getter_from_object_or_detached,
    boolean_attribute_property_getter_from_object_or_detached, property_dom_string_value,
    queue_text_track_load_if_source, reflected_attribute, resolve_url_like_attribute,
    set_dom_string_attribute_property_on_object, set_reflected_attribute,
    set_reflected_boolean_attribute, track_ready_state_for_handle,
};

#[derive(strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
enum TrackKind {
    Subtitles,
    Captions,
    Descriptions,
    Chapters,
    Metadata,
}

fn canonical_track_kind(value: &str) -> &'static str {
    TrackKind::from_str(value.trim())
        .map(Into::into)
        .unwrap_or("metadata")
}

pub(in crate::native_bridge::element) fn track_kind_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let kind = reflected_attribute(unsafe { &*runtime_ptr }, handle, "kind")
        .map(|value| canonical_track_kind(&value).to_owned())
        .unwrap_or_else(|| "subtitles".to_owned());
    if let Some(kind) = v8_string(scope, &kind) {
        rv.set(kind.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge::element) fn track_kind_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(next_value) =
        property_dom_string_value(scope, args.get(0), "HTMLTrackElement", "kind")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "kind", &next_value);
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn track_default_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "default", rv);
}

pub(in crate::native_bridge::element) fn track_default_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        "default",
        args.get(0).boolean_value(scope),
    );
    queue_text_track_load_if_source(scope, runtime_ptr, handle, "default");
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn track_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "src");
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge::element) fn track_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(src) = property_dom_string_value(scope, args.get(0), "HTMLTrackElement", "src") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &src);
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn track_srclang_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "srclang", rv);
}

pub(in crate::native_bridge::element) fn track_srclang_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "srclang",
        args.get(0),
        "HTMLTrackElement",
        "srclang",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn track_ready_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((_runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_uint32(0);
        return;
    };
    rv.set_uint32(track_ready_state_for_handle(scope, handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_track_kind_uses_html_track_tokens() {
        assert_eq!(canonical_track_kind("subtitles"), "subtitles");
        assert_eq!(canonical_track_kind("CAPTIONS"), "captions");
        assert_eq!(canonical_track_kind(" descriptions "), "descriptions");
        assert_eq!(canonical_track_kind("chapters"), "chapters");
        assert_eq!(canonical_track_kind("metadata"), "metadata");
        assert_eq!(canonical_track_kind("invalid"), "metadata");
    }
}

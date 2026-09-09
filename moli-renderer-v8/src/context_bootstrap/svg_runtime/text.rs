//! SVG text queries consume canonical glyph data from the same frozen layout
//! as other synchronous geometry APIs. No fixed-width estimates or live paint
//! resources are retained by this boundary.

use moli_svg::SvgTextQuery;

use super::*;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, node_runtime_and_handle_from_object_or_detached},
};

pub(super) fn is_text_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .is_some_and(|(host, handle)| {
            unsafe { &*host }
                .dom_host()
                .node(handle)
                .is_some_and(|node| {
                    node.namespace() == Some(moli_layout::LayoutNamespace::SVG_URI)
                        && matches!(node.local_name(), Some("text" | "tspan" | "textPath"))
                })
        })
}

pub(super) fn query_for_handle<T>(
    runtime: &JsContextHost,
    handle: DomHandle,
    inspect: impl FnOnce(SvgTextQuery<'_>) -> T,
) -> Option<T> {
    if !runtime.layout_policy().uses_real_layout()
        || !runtime.dom_host().node(handle)?.is_connected()
    {
        return None;
    }
    let document = runtime.layout_document_for_source(handle)?;
    if !runtime.can_answer_layout_from_snapshot(document) {
        // Only cold-start (or the explicit test policy) asks for a new pass.
        // A later DOM/style mutation does not refresh synchronous geometry.
        runtime
            .answer_layout_for_document(
                document,
                moli_layout::LayoutFlushReason::SynchronousGeometry,
                &moli_layout::LayoutQueryBatch::new(Vec::new()),
            )
            .ok()?;
    }
    runtime
        .with_latest_layout_tree_for_document(document, |tree| {
            tree.svg_text_for_source(handle).map(inspect)
        })
        .flatten()
}

fn query<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    inspect: impl FnOnce(SvgTextQuery<'_>) -> T,
) -> Option<T> {
    let (host, handle) = node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
    query_for_handle(unsafe { &*host }, handle, inspect)
}

fn index_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "IndexSizeError",
        1,
        "The character index is outside the rendered SVG text.",
    );
}

pub(super) fn svg_text_content_get_number_of_chars_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = query(scope, args.this(), |text| text.number_of_chars()).unwrap_or(0);
    rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
}

pub(super) fn computed_text_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> f64 {
    query(scope, receiver, |text| text.computed_text_length()).unwrap_or(0.0)
}

pub(super) fn svg_text_content_get_computed_text_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = computed_text_length(scope, args.this());
    rv.set(v8::Number::new(scope, f64::from(value as f32)).into());
}

pub(super) fn svg_text_content_get_substring_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextSubstringArgs>(scope, &args) else {
        return;
    };
    let Some(value) = query(scope, args.this(), |text| {
        text.substring_length(parsed.charnum as usize, parsed.nchars as usize)
    })
    .flatten() else {
        index_error(scope);
        return;
    };
    rv.set(v8::Number::new(scope, f64::from(value as f32)).into());
}

fn character<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<moli_svg::SvgTextCharacter> {
    let parsed = webidl::parse_args::<SvgTextCharacterIndexArgs>(scope, args)?;
    let value = query(scope, args.this(), |text| {
        text.character(parsed.charnum as usize)
    })
    .flatten();
    if value.is_none() {
        index_error(scope);
    }
    value
}

pub(super) fn svg_text_content_get_start_position_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = character(scope, &args) else {
        return;
    };
    rv.set(super::point::build(scope, value.start).into());
}

pub(super) fn svg_text_content_get_end_position_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = character(scope, &args) else {
        return;
    };
    rv.set(super::point::build(scope, value.end).into());
}

pub(super) fn svg_text_content_get_extent_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = character(scope, &args) else {
        return;
    };
    rv.set(super::rect::build_svg_rect(scope, value.extent).into());
}

pub(super) fn svg_text_content_get_rotation_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = character(scope, &args) else {
        return;
    };
    rv.set(v8::Number::new(scope, f64::from(value.rotation as f32)).into());
}

pub(super) fn svg_text_content_get_char_num_at_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(point) = optional_dom_point_init_arg(
        scope,
        &args,
        0,
        "SVGTextContentElement.getCharNumAtPosition",
    ) else {
        return;
    };
    let value = query(scope, args.this(), |text| {
        text.character_at_position(SvgGeometryPoint::new(point.x, point.y))
    })
    .flatten()
    .map_or(-1, |index| index as i32);
    rv.set(v8::Integer::new(scope, value).into());
}

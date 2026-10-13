pub(super) use moli_webapi::canvas_context_values::CANVAS_DRAWING_SLOTS;
use super::style_state::CanvasStyleColor;
use crate::native_bridge::{
    element::{StyleMode, style_property_value},
    node_runtime_and_handle_from_object_or_detached,
};
use crate::webidl;

pub(super) fn canvas_unrestricted_double_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Option<f64> {
    webidl::argument::<webidl::UnrestrictedDouble>(
        scope,
        args,
        index,
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
    .ok()
    .map(f64::from)
}

pub(super) fn canonical_canvas_color<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    raw: &str,
) -> Option<CanvasStyleColor> {
    // Resolve currentcolor after DOMString conversion. Author code can change
    // the canvas's document, connection or inherited color during conversion.
    let current = super::backing_store::canvas_owner_from_context(scope, context)
        .and_then(|canvas| node_runtime_and_handle_from_object_or_detached(scope, canvas).ok())
        .and_then(|(runtime, handle)| {
            let runtime = unsafe { &*runtime };
            if !runtime.node_is_connected_for_web_api(handle) {
                return None;
            }
            Some(style_property_value(
                runtime,
                handle,
                StyleMode::Computed,
                "color",
            ))
        });
    let color = moli_css_parse::parse_css_color_for_canvas(raw, current.as_deref())?;
    let color_space = super::settings::Canvas2dSettings::for_context(scope, context).color_space;
    let converted = color.absolute.to_color_space(color_space.stylo());
    Some(CanvasStyleColor::new(
        color.serialize(),
        [converted.components.0, converted.components.1, converted.components.2, converted.alpha],
    ))
}

pub(super) use moli_webapi::canvas_context_values::init_canvas_like_context_object;

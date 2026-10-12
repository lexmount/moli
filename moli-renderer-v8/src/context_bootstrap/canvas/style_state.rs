//! Native Canvas shadow and text attributes shared by Window and worker contexts.

use super::*;
use crate::{
    native_bridge::{
        element::{StyleMode, style_property_value},
        node_runtime_and_handle_from_object_or_detached,
    },
    util::{get_private_value, set_private_value},
};
use moli_webapi_declare::web_api_object_target;

macro_rules! style_state_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(accessor_property = "shadowBlur", getter = shadow_blur_getter, setter = shadow_blur_setter)]
            shadow_blur: (),
            #[webapi(accessor_property = "shadowColor", getter = shadow_color_getter, setter = shadow_color_setter)]
            shadow_color: (),
            #[webapi(accessor_property = "shadowOffsetX", getter = shadow_offset_x_getter, setter = shadow_offset_x_setter)]
            shadow_offset_x: (),
            #[webapi(accessor_property = "shadowOffsetY", getter = shadow_offset_y_getter, setter = shadow_offset_y_setter)]
            shadow_offset_y: (),
            #[webapi(accessor_property = "textAlign", getter = text_align_getter, setter = text_align_setter)]
            text_align: (),
            #[webapi(accessor_property = "textBaseline", getter = text_baseline_getter, setter = text_baseline_setter)]
            text_baseline: (),
        }
    };
}

style_state_declaration!(
    CanvasStyleStateDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
style_state_declaration!(
    OffscreenStyleStateDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_style_state_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => {
            CanvasStyleStateDeclaration::initialize_prototype_template(scope, prototype)
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenStyleStateDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("only 2D contexts have shadow and text attributes"),
    }
}

macro_rules! state_getter {
    ($name:ident, $slot:ident) => {
        fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            let context =
                web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
            rv.set(
                get_private_value(scope, context, $slot)
                    .expect("native drawing state is initialized"),
            );
        }
    };
}

state_getter!(shadow_blur_getter, CANVAS_CONTEXT_SHADOW_BLUR_SLOT);
state_getter!(shadow_color_getter, CANVAS_CONTEXT_SHADOW_COLOR_SLOT);
state_getter!(shadow_offset_x_getter, CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT);
state_getter!(shadow_offset_y_getter, CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT);
state_getter!(text_align_getter, CANVAS_CONTEXT_TEXT_ALIGN_SLOT);
state_getter!(text_baseline_getter, CANVAS_CONTEXT_TEXT_BASELINE_SLOT);

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasShadowStyles")]
struct ShadowNumberArgs {
    #[webidl(default = f64::NAN, converter = "unrestricted_double")]
    value: f64,
}

macro_rules! number_setter {
    ($name:ident, $slot:ident, $nonnegative:expr) => {
        fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            _rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            let context =
                web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
            let Some(ShadowNumberArgs { value }) = webidl::parse_args(scope, &args) else {
                return;
            };
            if value.is_finite() && (!$nonnegative || value >= 0.0) {
                let value = v8::Number::new(scope, value);
                set_private_value(scope, context, $slot, value.into());
            }
        }
    };
}

number_setter!(shadow_blur_setter, CANVAS_CONTEXT_SHADOW_BLUR_SLOT, true);
number_setter!(
    shadow_offset_x_setter,
    CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT,
    false
);
number_setter!(
    shadow_offset_y_setter,
    CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT,
    false
);

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D")]
struct StringArgs {
    #[webidl(default = "undefined".to_owned(), converter = "dom_string")]
    value: String,
}

fn shadow_color_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let Some(StringArgs { value }) = webidl::parse_args(scope, &args) else {
        return;
    };
    // Resolve after DOMString conversion: author code may change the canvas's
    // owner document or inherited color during conversion.
    let current = super::backing_store::canvas_owner_from_context(scope, context)
        .and_then(|canvas| node_runtime_and_handle_from_object_or_detached(scope, canvas).ok())
        .and_then(|(runtime, handle)| {
            let runtime = unsafe { &*runtime };
            // Canvas color resolution depends on DOM connection, including
            // windowless Documents and unslotted light DOM. CSSOM's stricter
            // rendered-flat-tree checks would reject those valid contexts.
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
    if let Some(color) = moli_css_parse::parse_css_color_for_canvas(&value, current.as_deref()) {
        set_string(scope, context, CANVAS_CONTEXT_SHADOW_COLOR_SLOT, &color);
    }
}

#[derive(strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum TextAlign {
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum TextBaseline {
    Top,
    Hanging,
    Middle,
    Alphabetic,
    Ideographic,
    Bottom,
}

fn text_align_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    enum_setter::<TextAlign>(scope, args, CANVAS_CONTEXT_TEXT_ALIGN_SLOT);
}

fn text_baseline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    enum_setter::<TextBaseline>(scope, args, CANVAS_CONTEXT_TEXT_BASELINE_SLOT);
}

fn enum_setter<'s, T: FromStr + Into<&'static str>>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    slot: &'static str,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let Some(StringArgs { value }) = webidl::parse_args(scope, &args) else {
        return;
    };
    // WebIDL enum attribute setters ignore unknown tokens, after ToString.
    // Conversion exceptions still propagate through the generated argument parser.
    if let Ok(value) = value.parse::<T>() {
        set_string(scope, context, slot, value.into());
    }
}

fn set_string(
    scope: &mut v8::PinScope<'_, '_>,
    context: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, context, slot, value.into());
    }
}

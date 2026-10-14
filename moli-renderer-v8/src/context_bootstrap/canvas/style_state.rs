//! Native Canvas color, shadow and text attributes shared across contexts.

use super::*;
use crate::util::{get_private_object, get_private_value, set_private_value, v8str};
use moli_webapi_declare::{WebApiObject, WebApiValue, web_api_object_target};

const COLOR_SERIALIZATION: &str = "__moliCanvasColorSerialization";
const COLOR_CHANNELS: &str = "__moliCanvasColorChannels";

#[derive(WebApiObject)]
#[webapi(plain)]
struct CanvasColorDeclaration {
    #[webapi(slot = COLOR_SERIALIZATION)]
    serialization: String,
    #[webapi(slot = COLOR_CHANNELS)]
    channels: [f32; 4],
}

/// Immutable native color value. save/restore traces this object without
/// reparsing a lossy CSS serialization or aliasing author-owned data.
pub(super) struct CanvasStyleColor {
    serialization: String,
    channels: [f32; 4],
}

impl CanvasStyleColor {
    pub(super) fn black() -> Self {
        Self {
            serialization: "#000000".to_owned(),
            channels: [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub(super) fn transparent() -> Self {
        Self {
            serialization: "rgba(0, 0, 0, 0)".to_owned(),
            channels: [0.0; 4],
        }
    }

    pub(super) fn new(
        color: moli_css_parse::CanvasColor,
        color_space: crate::context_bootstrap::image_data::ImageDataColorSpace,
    ) -> Self {
        let converted = color.absolute.to_color_space(color_space.stylo());
        Self {
            serialization: color.serialize(),
            channels: [
                converted.components.0,
                converted.components.1,
                converted.components.2,
                converted.alpha,
            ],
        }
    }
}

impl<'s> WebApiValue<'s> for CanvasStyleColor {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        CanvasColorDeclaration::new(self.serialization.clone(), self.channels)
            .bind(scope)
            .ok()
            .map(Into::into)
    }
}

pub(super) fn context_style_channels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    slot: &str,
) -> [f32; 4] {
    let color = get_private_object(scope, context, slot).expect("native color is initialized");
    let channels = get_private_value(scope, color, COLOR_CHANNELS)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("native color has immutable channels");
    std::array::from_fn(|index| {
        channels
            .get_index(scope, index as u32)
            .and_then(|value| value.number_value(scope))
            .expect("native color channel is a number") as f32
    })
}

pub(super) enum CanvasPaint {
    Solid([f32; 4]),
    Gradient(Box<super::gradient::GradientSnapshot>),
}

impl moli_canvas::PixelSource for CanvasPaint {
    fn sample(&self, x: u32, y: u32) -> [f32; 4] {
        match self {
            Self::Solid(color) => *color,
            Self::Gradient(gradient) => moli_canvas::PixelSource::sample(gradient.as_ref(), x, y),
        }
    }
}

pub(super) fn context_paint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    slot: &str,
    alpha: f32,
) -> CanvasPaint {
    let context = web_api_object_target(scope, context).expect("validated Canvas paint receiver");
    let object = get_private_object(scope, context, slot).expect("initialized Canvas style");
    if web_api_interfaces::CanvasGradient::is_instance(scope, object) {
        let data = super::state::canvas_gradient_state(scope, object);
        let transform = super::state::canvas_path_state(scope, context)
            .borrow()
            .transform();
        let space = super::settings::Canvas2dSettings::for_context(scope, context).color_space;
        return CanvasPaint::Gradient(Box::new(data.borrow().snapshot(space, transform, alpha)));
    }
    if web_api_interfaces::CanvasPattern::is_instance(scope, object) {
        // Pattern image producers are installed separately; there is currently
        // no public factory that can materialize this interface.
        return CanvasPaint::Solid([0.0; 4]);
    }
    let mut color = context_style_channels(scope, context, slot);
    color[3] *= alpha;
    CanvasPaint::Solid(color)
}

enum PaintStyleValue<'s> {
    Object(v8::Local<'s, v8::Object>),
    Color(String),
}

fn is_paint_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    [
        web_api_interfaces::CanvasGradient::DESCRIPTOR,
        web_api_interfaces::CanvasPattern::DESCRIPTOR,
    ]
    .iter()
    .any(|interface| interface.is_instance(scope, object))
}

impl<'s> webidl::WebIdlConverter<'s> for PaintStyleValue<'s> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && is_paint_object(scope, object)
        {
            return Ok(Self::Object(object));
        }
        Ok(Self::Color(
            webidl::convert::<webidl::DomString>(scope, value, context)?.0,
        ))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasFillStrokeStyles")]
struct PaintStyleArgs<'s> {
    #[webidl(with=paint_style_arg)]
    value: PaintStyleValue<'s>,
}

fn paint_style_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<PaintStyleValue<'s>, webidl::WebIdlError> {
    webidl::convert::<PaintStyleValue>(
        scope,
        args.get(index),
        webidl::Context::argument("CanvasFillStrokeStyles", (index + 1) as usize),
    )
}

fn paint_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context =
        web_api_object_target(scope, args.this()).expect("generated Canvas style receiver");
    let Some(parsed) = webidl::parse_args::<PaintStyleArgs>(scope, &args) else {
        return;
    };
    let slot = args.data().to_rust_string_lossy(scope);
    match parsed.value {
        PaintStyleValue::Object(object) => {
            set_private_value(scope, context, &slot, object.into());
        }
        PaintStyleValue::Color(raw) => {
            if let Some(color) = super::helpers::canonical_canvas_color(scope, context, &raw)
                && let Some(color) = color.to_v8_value(scope)
            {
                set_private_value(scope, context, &slot, color);
            }
        }
    }
}

macro_rules! style_state_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(accessor_property = "fillStyle", getter = color_getter, setter = paint_setter, data = v8str(scope, CANVAS_CONTEXT_FILL_STYLE_SLOT))]
            fill_style: (),
            #[webapi(accessor_property = "strokeStyle", getter = color_getter, setter = paint_setter, data = v8str(scope, CANVAS_CONTEXT_STROKE_STYLE_SLOT))]
            stroke_style: (),
            #[webapi(accessor_property = "shadowBlur", getter = shadow_blur_getter, setter = shadow_blur_setter)]
            shadow_blur: (),
            #[webapi(accessor_property = "shadowColor", getter = color_getter, setter = color_setter, data = v8str(scope, CANVAS_CONTEXT_SHADOW_COLOR_SLOT))]
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
        _ => unreachable!("only 2D contexts have drawing attributes"),
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

fn color_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let slot = args.data().to_rust_string_lossy(scope);
    let color = get_private_object(scope, context, &slot).expect("native color is initialized");
    if is_paint_object(scope, color) {
        rv.set(color.into());
        return;
    }
    rv.set(
        get_private_value(scope, color, COLOR_SERIALIZATION).expect("native color is serialized"),
    );
}

fn color_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let Some(StringArgs { value }) = webidl::parse_args(scope, &args) else {
        return;
    };
    if let Some(color) = super::helpers::canonical_canvas_color(scope, context, &value) {
        let slot = args.data().to_rust_string_lossy(scope);
        if let Some(color) = color.to_v8_value(scope) {
            set_private_value(scope, context, &slot, color);
        }
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
    slot: &str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, context, slot, value.into());
    }
}

//! Initial 2D context values and OffscreenCanvas construction. Drawing and
//! HTMLCanvasElement write-through remain embedding operations.
pub mod transform;

use crate::image_data::new_uint8_clamped_array_from_bytes;
use crate::interfaces as web_api_interfaces;
use moli_canvas::DEFAULT_FONT;
use moli_v8_util::{set_private_value, throw_type_error};
use moli_webapi_declare::{WebApiObject, WebApiValue};
use moli_webidl as webidl;
use std::str::FromStr;

pub const CANVAS_BACKING_STORE_SLOT: &str = "__moliCanvasBackingStore";
pub const OFFSCREEN_CANVAS_WIDTH_SLOT: &str = "__moliOffscreenCanvasWidth";
pub const OFFSCREEN_CANVAS_HEIGHT_SLOT: &str = "__moliOffscreenCanvasHeight";
pub const CANVAS_CONTEXT_FILL_STYLE_SLOT: &str = "__moliCanvasContextFillStyle";
pub const CANVAS_CONTEXT_FONT_SLOT: &str = "__moliCanvasContextFont";
pub const CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT: &str =
    "__moliCanvasContextImageSmoothingEnabled";
pub const CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT: &str =
    "__moliCanvasContextImageSmoothingQuality";
pub const CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT: &str = "__moliCanvasContextGlobalAlpha";
pub const CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT: &str =
    "__moliCanvasContextGlobalCompositeOperation";
pub const CANVAS_CONTEXT_LINE_WIDTH_SLOT: &str = "__moliCanvasContextLineWidth";
pub const CANVAS_CONTEXT_LINE_CAP_SLOT: &str = "__moliCanvasContextLineCap";
pub const CANVAS_CONTEXT_LINE_JOIN_SLOT: &str = "__moliCanvasContextLineJoin";
pub const CANVAS_CONTEXT_MITER_LIMIT_SLOT: &str = "__moliCanvasContextMiterLimit";
pub const CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT: &str = "__moliCanvasContextLineDashOffset";
pub const CANVAS_CONTEXT_STROKE_STYLE_SLOT: &str = "__moliCanvasContextStrokeStyle";
pub const CANVAS_CONTEXT_LINE_DASH_SLOT: &str = "__moliCanvasContextLineDash";
pub const CANVAS_CONTEXT_SHADOW_BLUR_SLOT: &str = "__moliCanvasContextShadowBlur";
pub const CANVAS_CONTEXT_SHADOW_COLOR_SLOT: &str = "__moliCanvasContextShadowColor";
pub const CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT: &str = "__moliCanvasContextShadowOffsetX";
pub const CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT: &str = "__moliCanvasContextShadowOffsetY";
pub const CANVAS_CONTEXT_TEXT_ALIGN_SLOT: &str = "__moliCanvasContextTextAlign";
pub const CANVAS_CONTEXT_TEXT_BASELINE_SLOT: &str = "__moliCanvasContextTextBaseline";

pub const DEFAULT_GLOBAL_ALPHA: f64 = 1.0;
pub const DEFAULT_GLOBAL_COMPOSITE_OPERATION: &str = "source-over";
pub const DEFAULT_LINE_WIDTH: f64 = 1.0;
pub const DEFAULT_LINE_CAP: &str = "butt";
pub const DEFAULT_LINE_JOIN: &str = "miter";
pub const DEFAULT_MITER_LIMIT: f64 = 10.0;
pub const DEFAULT_LINE_DASH_OFFSET: f64 = 0.0;

pub const COLOR_SERIALIZATION: &str = "__moliCanvasColorSerialization";
pub const COLOR_CHANNELS: &str = "__moliCanvasColorChannels";

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
pub struct CanvasStyleColor {
    serialization: String,
    channels: [f32; 4],
}

impl CanvasStyleColor {
    pub fn black() -> Self {
        Self {
            serialization: "#000000".to_owned(),
            channels: [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn transparent() -> Self {
        Self {
            serialization: "rgba(0, 0, 0, 0)".to_owned(),
            channels: [0.0; 4],
        }
    }

    pub fn new(serialization: String, channels: [f32; 4]) -> Self {
        Self { serialization, channels }
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

// Initialization, reset and save/restore use the same list of implemented
// drawing attributes. New native attributes must join this declaration.
macro_rules! canvas_drawing_slots {
    ($($name:ident: $ty:ty => $slot:ident = $default:expr;)*) => {
        #[derive(WebApiObject)]
        #[webapi(fragment, prototype = "CanvasRenderingContext2D")]
        struct CanvasLikeContextObjectDeclaration {
            $(#[webapi(slot = $slot, constructor_default = $default)]
            $name: $ty,)*
        }

        pub const CANVAS_DRAWING_SLOTS: &[&str] = &[$($slot),*];
    };
}

canvas_drawing_slots! {
    fill_style: CanvasStyleColor => CANVAS_CONTEXT_FILL_STYLE_SLOT = CanvasStyleColor::black();
    font: &'static str => CANVAS_CONTEXT_FONT_SLOT = DEFAULT_FONT;
    image_smoothing_enabled: bool => CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT = true;
    image_smoothing_quality: &'static str => CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT = "low";
    global_alpha: f64 => CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT = DEFAULT_GLOBAL_ALPHA;
    global_composite_operation: &'static str => CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT = DEFAULT_GLOBAL_COMPOSITE_OPERATION;
    line_width: f64 => CANVAS_CONTEXT_LINE_WIDTH_SLOT = DEFAULT_LINE_WIDTH;
    line_cap: &'static str => CANVAS_CONTEXT_LINE_CAP_SLOT = DEFAULT_LINE_CAP;
    line_join: &'static str => CANVAS_CONTEXT_LINE_JOIN_SLOT = DEFAULT_LINE_JOIN;
    miter_limit: f64 => CANVAS_CONTEXT_MITER_LIMIT_SLOT = DEFAULT_MITER_LIMIT;
    line_dash_offset: f64 => CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT = DEFAULT_LINE_DASH_OFFSET;
    stroke_style: CanvasStyleColor => CANVAS_CONTEXT_STROKE_STYLE_SLOT = CanvasStyleColor::black();
    line_dash: Vec<f64> => CANVAS_CONTEXT_LINE_DASH_SLOT = Vec::new();
    shadow_blur: f64 => CANVAS_CONTEXT_SHADOW_BLUR_SLOT = 0.0;
    shadow_color: CanvasStyleColor => CANVAS_CONTEXT_SHADOW_COLOR_SLOT = CanvasStyleColor::transparent();
    shadow_offset_x: f64 => CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT = 0.0;
    shadow_offset_y: f64 => CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT = 0.0;
    text_align: &'static str => CANVAS_CONTEXT_TEXT_ALIGN_SLOT = "start";
    text_baseline: &'static str => CANVAS_CONTEXT_TEXT_BASELINE_SLOT = "alphabetic";
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::OffscreenCanvas)]
struct OffscreenCanvasObjectDeclaration {
    #[webapi(slot = OFFSCREEN_CANVAS_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = OFFSCREEN_CANVAS_HEIGHT_SLOT)]
    height: f64,
}

pub fn init_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    CanvasLikeContextObjectDeclaration::new()
        .initialize(scope, object)
        .expect("canvas context declaration should initialize object");
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OffscreenCanvas")]
struct OffscreenCanvasConstructorArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    width: u32,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    height: u32,
}

pub fn offscreen_canvas_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'OffscreenCanvas': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<OffscreenCanvasConstructorArgs>(scope, &args) else {
        return;
    };
    init_offscreen_canvas_object(scope, args.this(), parsed.width, parsed.height);
    rv.set(args.this().into());
}

pub fn canvas_rendering_context_2d_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    init_canvas_like_context_object(scope, args.this());
    rv.set(args.this().into());
}

pub use canvas_rendering_context_2d_constructor_callback as offscreen_canvas_rendering_context_2d_constructor_callback;

fn init_offscreen_canvas_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    width: u32,
    height: u32,
) {
    OffscreenCanvasObjectDeclaration::new(f64::from(width), f64::from(height))
        .bind_into(scope, object)
        .expect("OffscreenCanvas declaration should initialize object");
    // A fresh OffscreenCanvas has no HTMLCanvasElement or attached context.
    // Allocate its initial pixels once; native drawing attaches its owner later.
    if let Some(bytes) = moli_canvas::byte_len(width, height)
        .and_then(|len| new_uint8_clamped_array_from_bytes(scope, vec![0; len]))
    {
        set_private_value(scope, object, CANVAS_BACKING_STORE_SLOT, bytes.into());
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "CanvasContextId", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
pub enum CanvasContextKind {
    #[strum(serialize = "2d")]
    TwoD,
    #[strum(serialize = "bitmaprenderer")]
    BitmapRenderer,
    WebGl,
    #[strum(serialize = "webgl2")]
    WebGl2,
    #[strum(serialize = "webgpu")]
    WebGpu,
}

impl CanvasContextKind {
    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod canvas_context_kind_tests {
    use super::CanvasContextKind;

    #[test]
    fn canvas_context_kind_parses_supported_context_ids() {
        assert_eq!(
            CanvasContextKind::parse("2d"),
            Some(CanvasContextKind::TwoD)
        );
        assert_eq!(
            CanvasContextKind::parse("webgl"),
            Some(CanvasContextKind::WebGl)
        );
        assert_eq!(
            CanvasContextKind::parse("webgl2"),
            Some(CanvasContextKind::WebGl2)
        );
        assert_eq!(CanvasContextKind::parse("WebGL"), None);
        assert_eq!(
            CanvasContextKind::parse("bitmaprenderer"),
            Some(CanvasContextKind::BitmapRenderer)
        );
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "ImageSmoothingQuality", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
pub enum ImageSmoothingQuality {
    Low,
    Medium,
    High,
}

impl ImageSmoothingQuality {
    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

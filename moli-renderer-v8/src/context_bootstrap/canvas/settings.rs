use crate::context_bootstrap::image_data::{ImageDataColorSpace, ImageDataPixelFormat};
use crate::{
    util::{get_private_value, new_null_prototype_object},
    webidl,
};
use moli_webapi_declare::WebApiObject;
use std::str::FromStr;

const ALPHA: &str = "__moliCanvas2dAlpha";
const COLOR_SPACE: &str = "__moliCanvas2dColorSpace";
const COLOR_TYPE: &str = "__moliCanvas2dColorType";
const WILL_READ_FREQUENTLY: &str = "__moliCanvas2dWillReadFrequently";

#[derive(Clone, Copy, Default, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "CanvasColorType", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum CanvasColorType {
    #[default]
    Unorm8,
    Float16,
}

impl CanvasColorType {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unorm8 => "unorm8",
            Self::Float16 => "float16",
        }
    }

    pub(super) fn pixel_format(self) -> ImageDataPixelFormat {
        match self {
            Self::Unorm8 => ImageDataPixelFormat::RgbaUnorm8,
            Self::Float16 => ImageDataPixelFormat::RgbaFloat16,
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlDictionary)]
#[webidl(prefix = "CanvasRenderingContext2DSettings")]
pub(crate) struct Canvas2dSettings {
    #[webidl(default = true)]
    pub(super) alpha: bool,
    #[webidl(converter = "enum", default = ImageDataColorSpace::Srgb)]
    pub(super) color_space: ImageDataColorSpace,
    #[webidl(converter = "enum", default = CanvasColorType::Unorm8)]
    color_type: CanvasColorType,
    #[webidl(name = "desynchronized", default = false)]
    _desynchronized: bool,
    #[webidl(default = false)]
    will_read_frequently: bool,
}

impl Default for Canvas2dSettings {
    fn default() -> Self {
        Self {
            alpha: true,
            color_space: ImageDataColorSpace::Srgb,
            color_type: CanvasColorType::Unorm8,
            _desynchronized: false,
            will_read_frequently: false,
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SettingsSlots {
    #[webapi(slot = ALPHA)]
    alpha: bool,
    #[webapi(slot = COLOR_SPACE)]
    color_space: String,
    #[webapi(slot = COLOR_TYPE)]
    color_type: String,
    #[webapi(slot = WILL_READ_FREQUENTLY)]
    will_read_frequently: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct Attributes {
    #[webapi(data_property, enumerable)]
    alpha: bool,
    #[webapi(data_property = "colorSpace", enumerable)]
    color_space: String,
    #[webapi(data_property = "colorType", enumerable)]
    color_type: String,
    #[webapi(data_property, enumerable)]
    desynchronized: bool,
    #[webapi(data_property = "willReadFrequently", enumerable)]
    will_read_frequently: bool,
}

pub(crate) fn parse_canvas_2d_settings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<Canvas2dSettings> {
    // The API accepts `any`; only object options participate in dictionary conversion.
    let options = v8::Local::<v8::Object>::try_from(options)
        .unwrap_or_else(|_| new_null_prototype_object(scope));
    match webidl::parse_dictionary_object::<Canvas2dSettings>(scope, options) {
        Ok(settings) => Some(settings),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

impl Canvas2dSettings {
    pub(super) fn initialize<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Object>,
    ) -> Option<()> {
        SettingsSlots::new(
            self.alpha,
            self.color_space.label().to_owned(),
            self.color_type.label().to_owned(),
            self.will_read_frequently,
        )
        .initialize(scope, context)
        .ok()
    }

    pub(super) fn for_context<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Object>,
    ) -> Self {
        Self {
            alpha: get_private_value(scope, context, ALPHA)
                .is_none_or(|value| value.boolean_value(scope)),
            color_space: get_private_value(scope, context, COLOR_SPACE)
                .and_then(|value| value.to_string(scope))
                .and_then(|value| value.to_rust_string_lossy(scope).parse().ok())
                .unwrap_or(ImageDataColorSpace::Srgb),
            color_type: get_private_value(scope, context, COLOR_TYPE)
                .and_then(|value| value.to_string(scope))
                .and_then(|value| value.to_rust_string_lossy(scope).parse().ok())
                .unwrap_or_default(),
            _desynchronized: false,
            will_read_frequently: get_private_value(scope, context, WILL_READ_FREQUENTLY)
                .is_some_and(|value| value.boolean_value(scope)),
        }
    }

    pub(super) fn pixel_format(self) -> ImageDataPixelFormat {
        self.color_type.pixel_format()
    }

    pub(super) fn attributes<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        // This CPU backing store does not offer a desynchronized presentation path.
        Attributes::new(
            self.alpha,
            self.color_space.label().to_owned(),
            self.color_type.label().to_owned(),
            false,
            self.will_read_frequently,
        )
        .bind(scope)
        .ok()
    }
}

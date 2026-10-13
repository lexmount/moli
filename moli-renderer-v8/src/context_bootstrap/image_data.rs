use super::*;
use crate::util::{callback_data_index_value, get_private_value};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};
use std::str::FromStr;

mod pixels;
use pixels::ImageDataArray;
pub(super) use pixels::{ImageDataPixelFormat, convert_pixel_bytes, new_pixel_array};

const IMAGE_DATA_WIDTH_SLOT: &str = "__moliImageDataWidth";
const IMAGE_DATA_HEIGHT_SLOT: &str = "__moliImageDataHeight";
const IMAGE_DATA_COLOR_SPACE_SLOT: &str = "__moliImageDataColorSpace";
const IMAGE_DATA_PIXEL_FORMAT_SLOT: &str = "__moliImageDataPixelFormat";
const IMAGE_DATA_DATA_SLOT: &str = "__moliImageDataData";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ImageData)]
struct ImageDataObjectDeclaration<'s> {
    #[webapi(slot = IMAGE_DATA_WIDTH_SLOT)]
    width: u32,
    #[webapi(slot = IMAGE_DATA_HEIGHT_SLOT)]
    height: u32,
    #[webapi(slot = IMAGE_DATA_COLOR_SPACE_SLOT)]
    color_space: String,
    #[webapi(slot = IMAGE_DATA_PIXEL_FORMAT_SLOT)]
    pixel_format: &'static str,
    #[webapi(slot = IMAGE_DATA_DATA_SLOT)]
    data: v8::Local<'s, v8::ArrayBufferView>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ImageData, enumerable, receiver)]
struct ImageDataPrototypeDeclaration {
    #[webapi(accessor_property, getter = image_data_getter_callback, data = callback_data_index_value(scope, 0))]
    width: (),
    #[webapi(accessor_property, getter = image_data_getter_callback, data = callback_data_index_value(scope, 1))]
    height: (),
    #[webapi(accessor_property, getter = image_data_getter_callback, data = callback_data_index_value(scope, 2))]
    data: (),
    #[webapi(accessor_property, getter = image_data_getter_callback, data = callback_data_index_value(scope, 3))]
    color_space: (),
    #[webapi(accessor_property, getter = image_data_getter_callback, data = callback_data_index_value(scope, 4))]
    pixel_format: (),
}

/// Retain the native view while V8 sub-serializes it in the enclosing object graph.
/// Copying bytes here would lose view identity, offsets and shared buffer references.
pub(crate) struct ImageDataClonePayload<'s> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) color_space: String,
    pub(crate) pixel_format: ImageDataPixelFormat,
    pub(crate) data: v8::Local<'s, v8::ArrayBufferView>,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "PredefinedColorSpace", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
pub(super) enum ImageDataColorSpace {
    Srgb,
    #[strum(serialize = "display-p3")]
    DisplayP3,
}

impl ImageDataColorSpace {
    pub(super) fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }
    pub(super) fn label(self) -> &'static str {
        self.into()
    }

    pub(super) fn stylo(self) -> style::color::ColorSpace {
        match self {
            Self::Srgb => style::color::ColorSpace::Srgb,
            Self::DisplayP3 => style::color::ColorSpace::DisplayP3,
        }
    }
}

#[derive(Clone, Copy, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ImageDataSettings")]
pub(super) struct ImageDataSettingsMembers {
    #[webidl(converter = "enum")]
    pub(super) color_space: Option<ImageDataColorSpace>,
    #[webidl(converter = "enum", default = ImageDataPixelFormat::RgbaUnorm8)]
    pub(super) pixel_format: ImageDataPixelFormat,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ImageData")]
struct ImageDataSizeArgs {
    #[webidl(required, converter = "unsigned_long")]
    width: u32,
    #[webidl(required, converter = "unsigned_long")]
    height: u32,
    #[webidl(dictionary)]
    settings: ImageDataSettingsMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ImageData")]
struct ImageDataSourceArgs<'s> {
    #[webidl(required, converter = "raw")]
    data: ImageDataArray<'s>,
    #[webidl(required, converter = "unsigned_long")]
    width: u32,
    #[webidl(converter = "unsigned_long")]
    height: Option<u32>,
    #[webidl(dictionary)]
    settings: ImageDataSettingsMembers,
}

pub(super) fn image_data_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ImageData': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parts) = image_data_constructor_parts(scope, &args) else {
        return;
    };
    declaration(parts)
        .initialize(scope, args.this())
        .expect("ImageData declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn install_image_data_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    ImageDataPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn image_data_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = web_api_object_target(scope, args.this()).expect("validated ImageData receiver");
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_DATA_ATTRIBUTE_SLOTS,
        "ImageData attribute slots",
    ) else {
        return;
    };
    if let Some(value) = get_private_value(scope, receiver, slot) {
        rv.set(value);
    }
}

fn image_data_constructor_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ImageDataClonePayload<'s>> {
    // Four arguments leave only the data overload. With two or three, the
    // distinguishing argument selects the native typed-array union or unsigned long.
    let (width, height, settings, data) =
        if args.length() >= 4 || ImageDataArray::matches(args.get(0)) {
            let parsed = webidl::parse_args::<ImageDataSourceArgs>(scope, args)?;
            let bytes_per_pixel = parsed.settings.pixel_format.bytes_per_pixel();
            let length = parsed.data.view.byte_length();
            if length == 0 || !length.is_multiple_of(bytes_per_pixel) {
                throw_invalid_image_data(scope);
                return None;
            }
            let pixels = length / bytes_per_pixel;
            if parsed.width == 0 || !pixels.is_multiple_of(parsed.width as usize) {
                webidl::throw_index_size_error(scope);
                return None;
            }
            let Ok(height) = u32::try_from(pixels / parsed.width as usize) else {
                webidl::throw_index_size_error(scope);
                return None;
            };
            if parsed.height.is_some_and(|given| given != height) {
                webidl::throw_index_size_error(scope);
                return None;
            }
            if parsed.data.format != parsed.settings.pixel_format {
                throw_invalid_image_data(scope);
                return None;
            }
            (parsed.width, height, parsed.settings, parsed.data.view)
        } else {
            let parsed = webidl::parse_args::<ImageDataSizeArgs>(scope, args)?;
            if parsed.width == 0 || parsed.height == 0 {
                webidl::throw_index_size_error(scope);
                return None;
            }
            let data = allocate_pixel_array(
                scope,
                parsed.width,
                parsed.height,
                parsed.settings.pixel_format,
            )?;
            (parsed.width, parsed.height, parsed.settings, data)
        };
    Some(ImageDataClonePayload {
        width,
        height,
        data,
        color_space: settings
            .color_space
            .unwrap_or(ImageDataColorSpace::Srgb)
            .label()
            .to_owned(),
        pixel_format: settings.pixel_format,
    })
}

fn allocate_pixel_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
    format: ImageDataPixelFormat,
) -> Option<v8::Local<'s, v8::ArrayBufferView>> {
    let Some(length) = format.byte_len(width, height) else {
        throw_image_data_allocation_error(scope);
        return None;
    };
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(length).is_err() {
        throw_image_data_allocation_error(scope);
        return None;
    }
    bytes.resize(length, 0);
    new_pixel_array(scope, bytes, format)
}

pub(super) fn throw_image_data_allocation_error(scope: &mut v8::PinScope<'_, '_>) {
    let message = crate::util::v8str(scope, "ImageData pixel storage could not be allocated.");
    let error = v8::Exception::range_error(scope, message);
    scope.throw_exception(error);
}

fn throw_invalid_image_data(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception_value(
        scope,
        "The pixel array does not match the ImageData format.",
        "InvalidStateError",
    );
}

const IMAGE_DATA_ATTRIBUTE_SLOTS: &[&str] = &[
    IMAGE_DATA_WIDTH_SLOT,
    IMAGE_DATA_HEIGHT_SLOT,
    IMAGE_DATA_DATA_SLOT,
    IMAGE_DATA_COLOR_SPACE_SLOT,
    IMAGE_DATA_PIXEL_FORMAT_SLOT,
];

pub(in crate::context_bootstrap) fn new_uint8_clamped_array_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8ClampedArray>> {
    let len = bytes.len();
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Uint8ClampedArray::new(scope, buffer, 0, len)
}

fn declaration(parts: ImageDataClonePayload<'_>) -> ImageDataObjectDeclaration<'_> {
    ImageDataObjectDeclaration::new(
        parts.width,
        parts.height,
        parts.color_space,
        parts.pixel_format.label(),
        parts.data,
    )
}

pub(super) fn build_image_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
    settings: ImageDataSettingsMembers,
) -> Option<v8::Local<'s, v8::Object>> {
    let format = settings.pixel_format;
    let data = allocate_pixel_array(scope, width, height, format)?;
    build_image_data_object_from_clone_payload(
        scope,
        ImageDataClonePayload {
            width,
            height,
            color_space: settings
                .color_space
                .unwrap_or(ImageDataColorSpace::Srgb)
                .label()
                .to_owned(),
            pixel_format: format,
            data,
        },
    )
}

pub(super) fn build_image_data_object_with_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut image: canvas::pixels::CanvasImage,
) -> Option<v8::Local<'s, v8::Object>> {
    image.pixels.unpremultiply();
    let width = image.pixels.width();
    let height = image.pixels.height();
    let color_space = image.color_space;
    let pixel_format = ImageDataPixelFormat::from_native(image.pixels.format());
    let data = image.into_view(scope)?;
    build_image_data_object_from_clone_payload(
        scope,
        ImageDataClonePayload {
            width,
            height,
            color_space: color_space.label().to_owned(),
            pixel_format,
            data,
        },
    )
}

pub(crate) fn build_image_data_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: ImageDataClonePayload<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    if payload.width == 0
        || payload.height == 0
        || payload
            .pixel_format
            .byte_len(payload.width, payload.height)?
            != payload.data.byte_length()
        || ImageDataArray::format(payload.data.into())? != payload.pixel_format
        || ImageDataColorSpace::parse(&payload.color_space).is_none()
    {
        return None;
    }
    declaration(payload).bind(scope).ok()
}

pub(crate) fn build_image_data_object_from_legacy_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
    color_space: String,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = new_uint8_clamped_array_from_bytes(scope, bytes)?;
    build_image_data_object_from_clone_payload(
        scope,
        ImageDataClonePayload {
            width,
            height,
            color_space,
            pixel_format: ImageDataPixelFormat::RgbaUnorm8,
            data: data.into(),
        },
    )
}

pub(crate) fn image_data_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<ImageDataClonePayload<'s>> {
    let object = web_api_object_target(scope, object)?;
    let (width, height) = image_data_dimensions_from_object(scope, object)?;
    let data = image_data_view(scope, object)?;
    if data.buffer(scope)?.was_detached() {
        return None;
    }
    let color_space =
        image_data_string_from_object_slot(scope, object, IMAGE_DATA_COLOR_SPACE_SLOT)?;
    let pixel_format =
        image_data_string_from_object_slot(scope, object, IMAGE_DATA_PIXEL_FORMAT_SLOT)?
            .parse()
            .ok()?;
    Some(ImageDataClonePayload {
        width,
        height,
        color_space,
        pixel_format,
        data,
    })
}

pub(super) fn image_data_dimensions_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
    Some((
        image_data_dimension_from_object_slot(scope, object, IMAGE_DATA_WIDTH_SLOT)?,
        image_data_dimension_from_object_slot(scope, object, IMAGE_DATA_HEIGHT_SLOT)?,
    ))
}

pub(super) fn image_data_settings_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<ImageDataSettingsMembers> {
    Some(ImageDataSettingsMembers {
        color_space: Some(ImageDataColorSpace::parse(
            &image_data_string_from_object_slot(scope, object, IMAGE_DATA_COLOR_SPACE_SLOT)?,
        )?),
        pixel_format: image_data_string_from_object_slot(
            scope,
            object,
            IMAGE_DATA_PIXEL_FORMAT_SLOT,
        )?
        .parse()
        .ok()?,
    })
}

fn image_data_dimension_from_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u32> {
    get_private_value(scope, object, slot)?.uint32_value(scope)
}

fn image_data_string_from_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, object, slot)?
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn image_data_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::ArrayBufferView>> {
    v8::Local::try_from(get_private_value(scope, object, IMAGE_DATA_DATA_SLOT)?).ok()
}

pub(super) fn image_data_buffer_is_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    Some(
        image_data_view(scope, object)?
            .buffer(scope)?
            .was_detached(),
    )
}

pub(super) fn image_data_pixel_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<canvas::pixels::CanvasImage> {
    let data = image_data_clone_payload_from_object(scope, object)?;
    canvas::pixels::CanvasImage::from_view(
        data.data,
        data.width,
        data.height,
        data.pixel_format,
        ImageDataColorSpace::parse(&data.color_space)?,
        moli_canvas::AlphaMode::Straight,
    )
}

#[cfg(test)]
mod image_data_color_space_tests {
    use super::ImageDataColorSpace;

    #[test]
    fn image_data_color_space_parses_supported_settings_token() {
        assert_eq!(
            ImageDataColorSpace::parse("srgb"),
            Some(ImageDataColorSpace::Srgb)
        );
        assert_eq!(ImageDataColorSpace::parse("SRGB"), None);
        assert_eq!(
            ImageDataColorSpace::parse("display-p3"),
            Some(ImageDataColorSpace::DisplayP3)
        );
    }

    #[test]
    fn image_data_color_space_label_uses_web_exposed_token() {
        assert_eq!(ImageDataColorSpace::Srgb.label(), "srgb");
        assert_eq!(ImageDataColorSpace::DisplayP3.label(), "display-p3");
    }
}

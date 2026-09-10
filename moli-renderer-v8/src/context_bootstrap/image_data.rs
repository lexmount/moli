use super::*;
use crate::util::{callback_data_index_value, get_private_value};
use crate::webidl;
use moli_canvas::byte_len as rgba8_byte_len;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::str::FromStr;

const IMAGE_DATA_WIDTH_SLOT: &str = "__moliImageDataWidth";
const IMAGE_DATA_HEIGHT_SLOT: &str = "__moliImageDataHeight";
const IMAGE_DATA_COLOR_SPACE_SLOT: &str = "__moliImageDataColorSpace";
const IMAGE_DATA_PIXEL_FORMAT_SLOT: &str = "__moliImageDataPixelFormat";
const IMAGE_DATA_DATA_SLOT: &str = "__moliImageDataData";
const IMAGE_DATA_BRAND_SLOT: &str = "__moliImageDataBrand";

#[derive(WebApiObject)]
#[webapi(interface = "ImageData")]
struct ImageDataObjectDeclaration<'s> {
    #[webapi(slot = IMAGE_DATA_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = IMAGE_DATA_WIDTH_SLOT)]
    width: u32,
    #[webapi(slot = IMAGE_DATA_HEIGHT_SLOT)]
    height: u32,
    #[webapi(slot = IMAGE_DATA_COLOR_SPACE_SLOT)]
    color_space: String,
    #[webapi(slot = IMAGE_DATA_PIXEL_FORMAT_SLOT)]
    pixel_format: &'static str,
    #[webapi(slot = IMAGE_DATA_DATA_SLOT)]
    data: v8::Local<'s, v8::Uint8ClampedArray>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ImageData")]
struct ImageDataPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = image_data_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = image_data_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = image_data_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    data: (),
    #[webapi(
        accessor_property,
        getter = image_data_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    color_space: (),
    #[webapi(
        accessor_property,
        getter = image_data_getter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    pixel_format: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ImageDataSettingsDeclaration<'scope> {
    color_space: v8::Local<'scope, v8::String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImageDataClonePayload {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) color_space: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "PredefinedColorSpace", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
enum ImageDataColorSpace {
    Srgb,
    #[strum(serialize = "display-p3")]
    DisplayP3,
}

impl ImageDataColorSpace {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ImageDataSettings")]
struct ImageDataSettingsMembers {
    #[webidl(converter = "enum", default = ImageDataColorSpace::Srgb)]
    color_space: ImageDataColorSpace,
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

    let Some((data, width, height, color_space)) = image_data_constructor_parts(scope, &args)
    else {
        return;
    };

    ImageDataObjectDeclaration::new(width, height, color_space, "rgba-unorm8", data)
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
    if !image_data_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_DATA_ATTRIBUTE_SLOTS,
        "ImageData attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value =
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn image_data_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, IMAGE_DATA_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn image_data_constructor_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(v8::Local<'s, v8::Uint8ClampedArray>, u32, u32, String)> {
    if let Ok(data) = v8::Local::<v8::Uint8ClampedArray>::try_from(args.get(0)) {
        let width = image_data_dimension_arg(scope, args.get(1))?;
        let height = if args.length() > 2 && !args.get(2).is_undefined() {
            image_data_dimension_arg(scope, args.get(2))?
        } else {
            let pixels = data.byte_length() / 4;
            if width == 0 || pixels % width as usize != 0 {
                throw_image_data_index_size_error(scope);
                return None;
            }
            (pixels / width as usize) as u32
        };
        let expected_len = image_data_expected_len(scope, width, height)?;
        if data.byte_length() != expected_len {
            throw_image_data_index_size_error(scope);
            return None;
        }
        let color_space = image_data_color_space_arg(scope, args.get(3))?;
        return Some((data, width, height, color_space));
    }

    let width = image_data_dimension_arg(scope, args.get(0))?;
    let height = image_data_dimension_arg(scope, args.get(1))?;
    let color_space = image_data_color_space_arg(scope, args.get(2))?;
    let len = image_data_expected_len(scope, width, height)?;
    let data = new_uint8_clamped_array_from_bytes(scope, vec![0; len])?;
    Some((data, width, height, color_space))
}

fn image_data_dimension_arg(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u32> {
    let number = value.number_value(scope)?;
    if !number.is_finite() || number <= 0.0 || number > i32::MAX as f64 {
        throw_image_data_index_size_error(scope);
        return None;
    }
    Some(number as u32)
}

fn image_data_color_space_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    if webidl::is_nullish(value) {
        return Some("srgb".to_owned());
    }
    let object = match v8::Local::<v8::Object>::try_from(value) {
        Ok(object) => object,
        Err(_) => {
            throw_type_error(
                scope,
                "Failed to construct 'ImageData': settings must be an object",
            );
            return None;
        }
    };
    let color_space =
        match webidl::parse_dictionary_object::<ImageDataSettingsMembers>(scope, object) {
            Ok(value) => value.color_space,
            Err(error) => {
                throw_type_error(scope, &error.to_string());
                return None;
            }
        };
    Some(color_space.label().to_owned())
}

const IMAGE_DATA_ATTRIBUTE_SLOTS: &[&str] = &[
    IMAGE_DATA_WIDTH_SLOT,
    IMAGE_DATA_HEIGHT_SLOT,
    IMAGE_DATA_DATA_SLOT,
    IMAGE_DATA_COLOR_SPACE_SLOT,
    IMAGE_DATA_PIXEL_FORMAT_SLOT,
];

fn image_data_expected_len(
    scope: &mut v8::PinScope<'_, '_>,
    width: u32,
    height: u32,
) -> Option<usize> {
    rgba8_byte_len(width, height).or_else(|| {
        throw_image_data_index_size_error(scope);
        None
    })
}

pub(in crate::context_bootstrap) fn new_uint8_clamped_array_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8ClampedArray>> {
    let len = bytes.len();
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Uint8ClampedArray::new(scope, buffer, 0, len)
}

pub(super) fn build_image_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor = v8::Local::<v8::Function>::try_from(super::shared::global_constructor_object(
        scope,
        "ImageData",
    )?)
    .ok()?;
    let object = ctor.new_instance(
        scope,
        &[
            v8::Integer::new(scope, width as i32).into(),
            v8::Integer::new(scope, height as i32).into(),
        ],
    )?;
    Some(object)
}

pub(super) fn build_image_data_object_with_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = new_uint8_clamped_array_from_bytes(scope, bytes)?;
    let ctor = v8::Local::<v8::Function>::try_from(super::shared::global_constructor_object(
        scope,
        "ImageData",
    )?)
    .ok()?;
    let object = ctor.new_instance(
        scope,
        &[
            data.into(),
            v8::Integer::new(scope, width as i32).into(),
            v8::Integer::new(scope, height as i32).into(),
        ],
    )?;
    Some(object)
}

pub(crate) fn build_image_data_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: ImageDataClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = new_uint8_clamped_array_from_bytes(scope, payload.bytes)?;
    let color_space = v8_string(scope, &payload.color_space)?;
    let settings = ImageDataSettingsDeclaration::new(color_space)
        .bind(scope)
        .expect("ImageData settings declaration should bind");
    let ctor = v8::Local::<v8::Function>::try_from(super::shared::global_constructor_object(
        scope,
        "ImageData",
    )?)
    .ok()?;
    ctor.new_instance(
        scope,
        &[
            data.into(),
            v8::Integer::new(scope, payload.width as i32).into(),
            v8::Integer::new(scope, payload.height as i32).into(),
            settings.into(),
        ],
    )
}

pub(crate) fn image_data_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<ImageDataClonePayload> {
    let (width, height) = image_data_dimensions_from_object(scope, object)?;
    let bytes = image_data_bytes_from_object(scope, object)?;
    if bytes.len() != rgba8_byte_len(width, height)? {
        return None;
    }
    let color_space =
        image_data_string_from_object_slot(scope, object, IMAGE_DATA_COLOR_SPACE_SLOT)?;
    Some(ImageDataClonePayload {
        width,
        height,
        color_space,
        bytes,
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

fn image_data_dimension_from_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u32> {
    let value = get_private_value(scope, object, slot)?.number_value(scope)?;
    if !value.is_finite() || value <= 0.0 || value > u32::MAX as f64 {
        return None;
    }
    Some(value as u32)
}

fn image_data_string_from_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, object, slot).and_then(|value| {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
    })
}

pub(super) fn image_data_bytes_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    let view = get_private_value(scope, object, IMAGE_DATA_DATA_SLOT)
        .and_then(|value| v8::Local::<v8::Uint8ClampedArray>::try_from(value).ok())?;
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    Some(bytes)
}

fn throw_image_data_index_size_error(scope: &mut v8::PinScope<'_, '_>) {
    crate::context_bootstrap::throw_dom_exception_value(
        scope,
        "The source width is zero or not a finite integer.",
        "IndexSizeError",
    );
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

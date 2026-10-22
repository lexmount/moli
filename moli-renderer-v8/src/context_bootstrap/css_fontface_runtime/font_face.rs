use super::*;
use crate::web_api_interfaces;
use crate::{
    util::{
        callback_data_index_value, callback_data_item, get_private_value, set_private_value,
        v8_string_from_utf16_units, v8_string_to_u16_string,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod descriptors;
use descriptors::{FONT_FACE_WRITABLE_ATTRIBUTES, parse_font_face_descriptors};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FontFace)]
struct FontFaceObjectDeclaration<'s> {
    #[webapi(slot = FONT_FACE_FAMILY_SLOT)]
    family: webidl::DomString16,
    #[webapi(slot = FONT_FACE_FAMILY_CSS_TEXT_SLOT)]
    family_css_text: bool,
    #[webapi(slot = FONT_FACE_SOURCE_SLOT)]
    source: String,
    #[webapi(slot = FONT_FACE_STYLE_SLOT)]
    style: String,
    #[webapi(slot = FONT_FACE_WEIGHT_SLOT)]
    weight: String,
    #[webapi(slot = FONT_FACE_STRETCH_SLOT)]
    stretch: String,
    #[webapi(slot = FONT_FACE_VARIANT_SLOT)]
    variant: String,
    #[webapi(slot = FONT_FACE_FEATURE_SETTINGS_SLOT)]
    feature_settings: String,
    #[webapi(slot = FONT_FACE_VARIATION_SETTINGS_SLOT)]
    variation_settings: String,
    #[webapi(slot = FONT_FACE_DISPLAY_SLOT)]
    display: String,
    #[webapi(slot = FONT_FACE_UNICODE_RANGE_SLOT)]
    unicode_range: String,
    #[webapi(slot = FONT_FACE_ASCENT_OVERRIDE_SLOT)]
    ascent_override: String,
    #[webapi(slot = FONT_FACE_DESCENT_OVERRIDE_SLOT)]
    descent_override: String,
    #[webapi(slot = FONT_FACE_LINE_GAP_OVERRIDE_SLOT)]
    line_gap_override: String,
    #[webapi(slot = FONT_FACE_SIZE_ADJUST_SLOT)]
    size_adjust: String,
    #[webapi(slot = FONT_FACE_LOADED_SLOT)]
    loaded: Option<v8::Local<'s, v8::Promise>>,
    #[webapi(slot = FONT_FACE_SET_OWNERS_SLOT, constructor_default = Vec::new())]
    owner_sets: Vec<v8::Local<'s, v8::Value>>,
    #[webapi(slot = FONT_FACE_LOAD_NOTIFICATION_SENT_SLOT, constructor_default = false)]
    load_notification_sent: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FontFace, receiver)]
struct FontFacePrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = font_face_family_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    family: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    style: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    weight: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    stretch: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    variant: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    feature_settings: (),
    #[webapi(
        accessor_property = "variationSettings",
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable
    )]
    variation_settings: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable
    )]
    display: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 8),
        enumerable
    )]
    unicode_range: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 9),
        enumerable
    )]
    ascent_override: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 10),
        enumerable
    )]
    descent_override: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 11),
        enumerable
    )]
    line_gap_override: (),
    #[webapi(
        accessor_property,
        getter = font_face_writable_attribute_getter_callback,
        setter = font_face_attribute_setter_callback,
        data = callback_data_index_value(scope, 12),
        enumerable
    )]
    size_adjust: (),
    #[webapi(
        accessor_property,
        getter = font_face_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    source: (),
    #[webapi(
        accessor_property,
        getter = font_face_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    status: (),
    #[webapi(
        accessor_property,
        getter = font_face_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        returns_promise,
        enumerable
    )]
    loaded: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FontFace")]
struct FontFaceConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    family: webidl::DomString16,
    #[webidl(required, with = font_face_constructor_source_arg)]
    source: FontFaceConstructorSource<'s>,
}

enum FontFaceConstructorSource<'s> {
    Css(String),
    Binary(v8::Local<'s, v8::Value>),
}

pub(in crate::context_bootstrap) fn install_font_face_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FontFacePrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn font_face_family_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = font_face_slot_value(scope, args.this(), FONT_FACE_FAMILY_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    if font_face_slot_value(scope, args.this(), FONT_FACE_FAMILY_CSS_TEXT_SLOT)
        .is_some_and(|value| value.is_true())
        && let Ok(name) = v8::Local::<v8::String>::try_from(value)
    {
        let name = v8_string_to_u16_string(scope, name);
        let serialized = moli_css_parse::serialize_font_family_name_utf16(name.as_slice());
        if let Some(value) = v8_string_from_utf16_units(scope, &serialized) {
            rv.set(value.into());
        }
        return;
    }
    rv.set(value);
}

fn font_face_writable_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(descriptor) = callback_data_item(
        scope,
        &args,
        FONT_FACE_WRITABLE_ATTRIBUTES,
        "FontFace writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = font_face_slot_value(scope, args.this(), descriptor.slot)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn font_face_readonly_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        FONT_FACE_READONLY_ATTRIBUTE_SLOTS,
        "FontFace readonly attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    if slot == FONT_FACE_STATUS_SLOT {
        let status = super::loading::status(scope, args.this());
        if let Some(value) = v8_string(scope, status.as_str()) {
            rv.set(value.into());
        }
        return;
    }
    if slot == FONT_FACE_LOADED_SLOT {
        if let Some(promise) = super::loading::loaded_promise(scope, args.this()) {
            rv.set(promise.into());
        }
        return;
    }
    let value = font_face_slot_value(scope, args.this(), slot)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn font_face_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(descriptor) = callback_data_item(
        scope,
        &args,
        FONT_FACE_WRITABLE_ATTRIBUTES,
        "FontFace writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = if descriptor.slot == FONT_FACE_FAMILY_SLOT {
        value
    } else {
        let Some(parsed) = descriptor.parse(&value.to_rust_string_lossy(scope)) else {
            webidl::throw_dom_exception(scope, "SyntaxError", "Invalid FontFace descriptor.");
            return;
        };
        let Some(value) = v8_string(scope, &parsed) else {
            return;
        };
        value
    };
    set_font_face_slot_value(scope, args.this(), descriptor.slot, value.into());
    super::loading::update_descriptor(scope, args.this());
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn font_face_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Constructor must be called with new");
        return;
    }
    let request_context = crate::network_host::capture_font_face_request_context(scope);
    let Some(parsed) = webidl::parse_args::<FontFaceConstructorArgs>(scope, &args) else {
        return;
    };
    let Some((descriptors, invalid_descriptors)) = parse_font_face_descriptors(scope, args.get(2))
    else {
        return;
    };
    let [
        style,
        weight,
        stretch,
        variant,
        feature_settings,
        variation_settings,
        display,
        unicode_range,
        ascent_override,
        descent_override,
        line_gap_override,
        size_adjust,
    ] = descriptors;
    let this = args.this();
    let family_css_text = matches!(&parsed.source, FontFaceConstructorSource::Css(_))
        && moli_css_parse::font_family_name_needs_quotes(&parsed.family.0);
    let (source, sources, binary) = match parsed.source {
        FontFaceConstructorSource::Css(source) => {
            let sources = moli_css_parse::parse_font_face_sources(&source).map(|sources| {
                crate::network_host::resolve_font_face_sources(scope, request_context, sources)
            });
            (source, sources, None)
        }
        FontFaceConstructorSource::Binary(value) => {
            // Preserve main's conversion order: descriptor getters may detach
            // the retained buffer, and invalid descriptors reject before copying.
            let bytes = if invalid_descriptors {
                None
            } else {
                match webidl::convert::<webidl::BufferSource>(
                    scope,
                    value,
                    webidl::Context::argument("FontFace", 2),
                ) {
                    Ok(bytes) => Some(bytes.into_bytes()),
                    Err(error) => {
                        webidl::throw_error(scope, &error);
                        return;
                    }
                }
            };
            (String::new(), Some(Vec::new()), bytes)
        }
    };
    FontFaceObjectDeclaration::new(
        parsed.family,
        family_css_text,
        source,
        style,
        weight,
        stretch,
        variant,
        feature_settings,
        variation_settings,
        display,
        unicode_range,
        ascent_override,
        descent_override,
        line_gap_override,
        size_adjust,
        None,
    )
    .initialize(scope, this)
    .expect("FontFace declaration should initialize object");
    let valid_sources = sources.is_some();
    let resource = super::loading::new_resource(scope, this, sources.unwrap_or_default());
    if invalid_descriptors {
        resource
            .borrow_mut()
            .fail("SyntaxError", "Invalid FontFace descriptor.");
    } else if !valid_sources {
        resource
            .borrow_mut()
            .fail("SyntaxError", "Invalid FontFace source.");
    } else if let Some(bytes) = binary {
        resource.borrow_mut().initialize_binary(&bytes);
    }
    super::loading::initialize(scope, this, resource, request_context);
    rv.set(this.into());
}

fn font_face_constructor_source_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<FontFaceConstructorSource<'s>, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to construct 'FontFace': 2 arguments required, but only 1 present.",
        ));
    }
    let value = args.get(index);
    let context = webidl::Context::argument("FontFace", (index + 1) as usize);
    // The BufferSource alternative has neither AllowShared nor AllowResizable.
    if value.is_shared_array_buffer()
        || crate::blob::buffer_source_has_shared_or_resizable_backing_store(value)
    {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to construct 'FontFace': shared and resizable buffers are not allowed.",
        ));
    }
    if v8::Local::<v8::ArrayBuffer>::try_from(value).is_ok()
        || v8::Local::<v8::ArrayBufferView>::try_from(value).is_ok()
    {
        return Ok(FontFaceConstructorSource::Binary(value));
    }
    webidl::convert::<webidl::DomString>(scope, value, context)
        .map(|source| FontFaceConstructorSource::Css(source.into()))
}

const FONT_FACE_READONLY_ATTRIBUTE_SLOTS: &[&str] = &[
    FONT_FACE_SOURCE_SLOT,
    FONT_FACE_STATUS_SLOT,
    FONT_FACE_LOADED_SLOT,
];

pub(in crate::context_bootstrap) fn font_face_load_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    match super::loading::load_font(scope, args.this()) {
        Some(promise) => rv.set(v8::Local::<v8::Value>::from(promise)),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn font_face_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot)
}

fn set_font_face_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, object, slot, value);
}

pub(crate) fn mark_font_face_css_connected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    let value = v8::Boolean::new(scope, false);
    set_font_face_slot_value(scope, face, FONT_FACE_FAMILY_CSS_TEXT_SLOT, value.into());
}

pub(crate) fn font_face_raw_family<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<moli_dom::native::DomStringValue> {
    let name = font_face_slot_value(scope, face, FONT_FACE_FAMILY_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())?;
    let units = v8_string_to_u16_string(scope, name);
    Some(moli_dom::native::DomStringValue::from_utf16(
        units.as_slice(),
    ))
}

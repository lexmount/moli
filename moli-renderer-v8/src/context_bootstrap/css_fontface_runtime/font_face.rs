use super::*;
use crate::web_api_interfaces;
use crate::{
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod descriptors;
use descriptors::{FONT_FACE_WRITABLE_ATTRIBUTES, parse_font_face_descriptors};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FontFace)]
struct FontFaceObjectDeclaration<'s> {
    #[webapi(slot = FONT_FACE_FAMILY_SLOT)]
    family: String,
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
    #[webapi(slot = FONT_FACE_STATUS_SLOT)]
    status: &'static str,
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
        getter = font_face_writable_attribute_getter_callback,
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
struct FontFaceConstructorArgs {
    #[webidl(required)]
    family: String,
    #[webidl(required, with = font_face_constructor_source_arg)]
    source: FontFaceConstructorSource,
}

enum FontFaceConstructorSource {
    Css(String),
    Binary(Vec<u8>),
}

pub(in crate::context_bootstrap) fn install_font_face_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FontFacePrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
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
    let (source, status, loaded) = if invalid_descriptors {
        let source = match parsed.source {
            FontFaceConstructorSource::Css(source) => source,
            FontFaceConstructorSource::Binary(_) => String::new(),
        };
        let loaded = super::query::make_rejected_dom_exception_promise(
            scope,
            "SyntaxError",
            "Invalid FontFace descriptor.",
        );
        (source, "error", Some(loaded))
    } else {
        match parsed.source {
            FontFaceConstructorSource::Css(source) => {
                let loaded = resolved_promise(scope, this.into());
                (source, "loaded", loaded)
            }
            FontFaceConstructorSource::Binary(bytes)
                if moli_web_mime::sniff_font_mime_type(&bytes).is_some() =>
            {
                let loaded = resolved_promise(scope, this.into());
                (String::new(), "loaded", loaded)
            }
            FontFaceConstructorSource::Binary(_) => {
                let loaded = super::query::make_rejected_dom_exception_promise(
                    scope,
                    "SyntaxError",
                    "Invalid font data in ArrayBuffer.",
                );
                (String::new(), "error", Some(loaded))
            }
        }
    };
    FontFaceObjectDeclaration::new(
        parsed.family,
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
        status,
        loaded,
    )
    .initialize(scope, this)
    .expect("FontFace declaration should initialize object");
    rv.set(this.into());
}

fn font_face_constructor_source_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<FontFaceConstructorSource, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to construct 'FontFace': 2 arguments required, but only 1 present.",
        ));
    }
    let value = args.get(index);
    let context = webidl::Context::argument("FontFace", (index + 1) as usize);
    if v8::Local::<v8::ArrayBuffer>::try_from(value).is_ok()
        || v8::Local::<v8::ArrayBufferView>::try_from(value).is_ok()
    {
        return webidl::convert::<webidl::BufferSource>(scope, value, context)
            .map(|source| FontFaceConstructorSource::Binary(source.into_bytes()));
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
    let this = args.this();
    super::events::notify_font_face_set_owners_of_load(scope, this);
    if let Some(loaded) = font_face_slot_value(scope, this, FONT_FACE_LOADED_SLOT) {
        rv.set(loaded);
        return;
    }
    match resolved_promise(scope, this.into()) {
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

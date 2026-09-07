use super::*;
use crate::{
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = "FontFace")]
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
#[webapi(name = "FontFace", receiver = is_font_face)]
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
    let Some(slot) = callback_data_item(
        scope,
        &args,
        FONT_FACE_WRITABLE_ATTRIBUTE_SLOTS,
        "FontFace writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = font_face_slot_value(scope, args.this(), slot)
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
    let Some(slot) = callback_data_item(
        scope,
        &args,
        FONT_FACE_WRITABLE_ATTRIBUTE_SLOTS,
        "FontFace writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = args
        .get(0)
        .to_string(scope)
        .unwrap_or_else(|| v8::String::empty(scope));
    let descriptor_name = match slot {
        FONT_FACE_FAMILY_SLOT => Some("font-family"),
        FONT_FACE_STYLE_SLOT => Some("font-style"),
        FONT_FACE_WEIGHT_SLOT => Some("font-weight"),
        FONT_FACE_STRETCH_SLOT => Some("font-stretch"),
        FONT_FACE_VARIATION_SETTINGS_SLOT => Some("font-variation-settings"),
        _ => None,
    };
    let value = if let Some(descriptor_name) = descriptor_name {
        let raw = value.to_rust_string_lossy(scope);
        let Some(value) = canonical_font_face_descriptor_value(descriptor_name, &raw) else {
            webidl::throw_dom_exception(
                scope,
                "SyntaxError",
                "Invalid FontFace variationSettings descriptor.",
            );
            return;
        };
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into()
    } else {
        value.into()
    };
    set_font_face_slot_value(scope, args.this(), slot, value);
    super::loading::sync_owners(scope, args.this());
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
    let descriptors = v8::Local::<v8::Object>::try_from(args.get(2)).ok();
    let this = args.this();
    let style = descriptor_string_property(scope, descriptors, "style", "normal");
    let weight = descriptor_string_property(scope, descriptors, "weight", "normal");
    let stretch = descriptor_string_property(scope, descriptors, "stretch", "normal");
    let variant = descriptor_string_property(scope, descriptors, "variant", "normal");
    let feature_settings =
        descriptor_string_property(scope, descriptors, "featureSettings", "normal");
    let Some(variation_settings) = descriptor_variation_settings_property(scope, descriptors)
    else {
        return;
    };
    let display = descriptor_string_property(scope, descriptors, "display", "auto");
    let valid_descriptors = [
        ("font-family", parsed.family.as_str()),
        ("font-style", style.as_str()),
        ("font-weight", weight.as_str()),
        ("font-stretch", stretch.as_str()),
    ]
    .iter()
    .all(|(name, value)| canonical_font_face_descriptor_value(name, value).is_some());
    let (source, terminal) = match parsed.source {
        FontFaceConstructorSource::Css(source) => {
            let terminal = moli_css_parse::parse_font_face_sources(&source)
                .is_none()
                .then_some(Err(("SyntaxError", "Invalid FontFace source.")));
            (source, terminal)
        }
        FontFaceConstructorSource::Binary(bytes) => (
            String::new(),
            Some(
                moli_layout::FontFaceData::from_bytes(&bytes)
                    .map_err(|_| ("SyntaxError", "Invalid font data in ArrayBuffer.")),
            ),
        ),
    };
    let resolver = v8::PromiseResolver::new(scope).expect("FontFace loaded resolver");
    let loaded = Some(resolver.get_promise(scope));
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
        "unloaded",
        loaded,
    )
    .initialize(scope, this)
    .expect("FontFace declaration should initialize object");
    super::loading::initialize_resolver(scope, this, resolver);
    if !valid_descriptors {
        super::loading::settle_font(
            scope,
            this,
            Err(("SyntaxError", "Invalid FontFace descriptor.")),
        );
    } else if let Some(terminal) = terminal {
        super::loading::settle_font(scope, this, terminal);
    }
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

const FONT_FACE_WRITABLE_ATTRIBUTE_SLOTS: &[&str] = &[
    FONT_FACE_FAMILY_SLOT,
    FONT_FACE_STYLE_SLOT,
    FONT_FACE_WEIGHT_SLOT,
    FONT_FACE_STRETCH_SLOT,
    FONT_FACE_VARIANT_SLOT,
    FONT_FACE_FEATURE_SETTINGS_SLOT,
    FONT_FACE_VARIATION_SETTINGS_SLOT,
    FONT_FACE_DISPLAY_SLOT,
];

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

pub(in crate::context_bootstrap) fn is_font_face<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, FONT_FACE_STATUS_SLOT).is_some()
}

fn descriptor_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: Option<v8::Local<'_, v8::Object>>,
    key: &str,
    default: &str,
) -> String {
    object
        .and_then(|object| v8_string(scope, key).and_then(|key| object.get(scope, key.into())))
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| default.to_owned())
}

fn descriptor_variation_settings_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: Option<v8::Local<'_, v8::Object>>,
) -> Option<String> {
    let Some(value) = object
        .and_then(|object| {
            v8_string(scope, "variationSettings").and_then(|key| object.get(scope, key.into()))
        })
        .filter(|value| !value.is_undefined())
    else {
        return Some("normal".to_owned());
    };
    let value = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))?;
    let Some(value) = canonical_font_face_descriptor_value("font-variation-settings", &value)
    else {
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            "Invalid FontFace variationSettings descriptor.",
        );
        return None;
    };
    Some(value)
}

fn canonical_font_face_descriptor_value(name: &str, value: &str) -> Option<String> {
    moli_css_parse::parse_font_face_descriptor_entry_with_stylo(name, value)
        .map(|entry| entry.value)
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

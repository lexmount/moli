use super::*;
use crate::{
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use data_url::DataUrl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use url::Url;

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
#[webapi(name = "FontFace")]
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
    let value = if slot == FONT_FACE_VARIATION_SETTINGS_SLOT {
        let raw = value.to_rust_string_lossy(scope);
        let Some(value) = canonical_font_face_descriptor_value("font-variation-settings", &raw)
        else {
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
    let (source, status, loaded) = match parsed.source {
        FontFaceConstructorSource::Css(source)
            if moli_css_parse::normalize_font_face_src(&source).is_none() =>
        {
            let loaded = super::query::make_rejected_dom_exception_promise(
                scope,
                "SyntaxError",
                "Invalid FontFace source descriptor.",
            );
            (source, "error", Some(loaded))
        }
        FontFaceConstructorSource::Css(source)
            if font_face_data_url_bytes(&source).is_some_and(|bytes| {
                bytes
                    .as_deref()
                    .is_none_or(|bytes| moli_layout::validate_web_font_bytes(bytes).is_err())
            }) =>
        {
            let loaded = super::query::make_rejected_dom_exception_promise(
                scope,
                "NetworkError",
                "The FontFace data URL does not contain a valid font.",
            );
            (source, "error", Some(loaded))
        }
        FontFaceConstructorSource::Css(source) => {
            let loaded = resolved_promise(scope, this.into());
            (source, "loaded", loaded)
        }
        FontFaceConstructorSource::Binary(bytes)
            if moli_layout::validate_web_font_bytes(&bytes).is_ok() =>
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
        status,
        loaded,
    )
    .initialize(scope, this)
    .expect("FontFace declaration should initialize object");
    rv.set(this.into());
}

fn font_face_data_url_bytes(source: &str) -> Option<Option<Vec<u8>>> {
    let normalized = moli_css_parse::normalize_font_face_src(source)?;
    let base_url = Url::parse("about:blank").expect("static FontFace base URL should parse");
    let request_url = crate::css_resource_urls::preferred_font_source_url(&normalized, &base_url)?;
    if request_url.scheme() != "data" {
        return None;
    }
    Some(
        DataUrl::process(request_url.as_str())
            .ok()
            .and_then(|data_url| data_url.decode_to_vec().ok())
            .map(|(bytes, _)| bytes),
    )
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

fn descriptor_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: Option<v8::Local<'_, v8::Object>>,
    key: &str,
    default: &str,
) -> String {
    object
        .and_then(|object| v8_string(scope, key).and_then(|key| object.get(scope, key.into())))
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

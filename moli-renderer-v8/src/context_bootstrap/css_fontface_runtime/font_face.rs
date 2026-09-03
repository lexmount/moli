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
    #[webapi(slot = FONT_FACE_LOADED_RESOLVER_SLOT)]
    loaded_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    #[webapi(slot = FONT_FACE_ERROR_SLOT)]
    error: Option<v8::Local<'s, v8::Value>>,
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
        getter = font_face_loaded_getter_callback,
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

fn font_face_loaded_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(loaded) = ensure_font_face_loaded_promise(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    rv.set(loaded.into());
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
    let (source, status, error) = match parsed.source {
        FontFaceConstructorSource::Css(source)
            if moli_css_parse::normalize_font_face_src(&source).is_some() =>
        {
            (source, "unloaded", None)
        }
        FontFaceConstructorSource::Css(source) => {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "Invalid FontFace source descriptor.",
                "SyntaxError",
            );
            (source, "error", Some(error))
        }
        FontFaceConstructorSource::Binary(bytes)
            if moli_web_mime::sniff_font_mime_type(&bytes).is_some() =>
        {
            (String::new(), "loaded", None)
        }
        FontFaceConstructorSource::Binary(_) => {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "Invalid font data in ArrayBuffer.",
                "SyntaxError",
            );
            (String::new(), "error", Some(error))
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
        None,
        None,
        error,
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

const FONT_FACE_READONLY_ATTRIBUTE_SLOTS: &[&str] = &[FONT_FACE_SOURCE_SLOT, FONT_FACE_STATUS_SLOT];

pub(in crate::context_bootstrap) fn font_face_load_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = args.this();
    let loaded = ensure_font_face_loaded_promise(scope, this);
    start_font_face_load(scope, this);
    if let Some(loaded) = loaded {
        rv.set(loaded.into());
        return;
    }
    match resolved_promise(scope, this.into()) {
        Some(promise) => rv.set(v8::Local::<v8::Value>::from(promise)),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(super) fn start_font_face_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    if font_face_status(scope, face).as_deref() != Some("unloaded") {
        super::events::notify_font_face_set_owners_of_load(scope, face);
        return;
    }

    set_font_face_status(scope, face, "loading");
    let source = font_face_string_slot(scope, face, FONT_FACE_SOURCE_SLOT).unwrap_or_default();
    let succeeds = font_face_css_source_has_url(&source);
    if succeeds {
        set_font_face_status(scope, face, "loaded");
    } else {
        set_font_face_status(scope, face, "error");
    }

    if succeeds {
        if let Some(resolver) = font_face_loaded_resolver(scope, face) {
            let _ = resolver.resolve(scope, face.into());
        }
    } else {
        let exception = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "No source in the FontFace src list could be loaded.",
            "NetworkError",
        );
        set_font_face_slot_value(scope, face, FONT_FACE_ERROR_SLOT, exception);
        if let Some(resolver) = font_face_loaded_resolver(scope, face) {
            let _ = resolver.reject(scope, exception);
        }
    }
    super::events::notify_font_face_set_owners_of_load(scope, face);
}

pub(super) fn font_face_load_failed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> bool {
    font_face_status(scope, face).as_deref() == Some("error")
}

pub(crate) fn load_font_faces_for_family<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    font_set: v8::Local<'s, v8::Object>,
    family: &str,
) {
    let Some(faces) = super::storage::font_face_set_faces_array(scope, font_set) else {
        return;
    };
    for index in 0..faces.length() {
        let Some(face) = faces
            .get_index(scope, index)
            .and_then(|face| v8::Local::<v8::Object>::try_from(face).ok())
        else {
            continue;
        };
        if !font_face_string_slot(scope, face, FONT_FACE_FAMILY_SLOT)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(family))
        {
            continue;
        }
        start_font_face_load(scope, face);
    }
}

fn font_face_css_source_has_url(source: &str) -> bool {
    moli_css_parse::normalize_font_face_src(source)
        .and_then(|source| crate::css_style::top_level_comma_separated_component_values(&source))
        .is_some_and(|sources| {
            sources
                .iter()
                .any(|source| source.trim_start().starts_with("url("))
        })
}

fn font_face_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<String> {
    font_face_string_slot(scope, face, FONT_FACE_STATUS_SLOT)
}

fn font_face_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    font_face_slot_value(scope, face, slot)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_font_face_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    status: &'static str,
) {
    let status = v8_string(scope, status).unwrap_or_else(|| v8::String::empty(scope));
    set_font_face_slot_value(scope, face, FONT_FACE_STATUS_SLOT, status.into());
}

fn font_face_loaded_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    font_face_slot_value(scope, face, FONT_FACE_LOADED_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

fn ensure_font_face_loaded_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if let Some(loaded) = font_face_slot_value(scope, face, FONT_FACE_LOADED_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    {
        return Some(loaded);
    }
    let status = font_face_status(scope, face)?;
    let resolver = v8::PromiseResolver::new(scope)?;
    let loaded = resolver.get_promise(scope);
    set_font_face_slot_value(scope, face, FONT_FACE_LOADED_SLOT, loaded.into());
    match status.as_str() {
        "loaded" => {
            let _ = resolver.resolve(scope, face.into());
        }
        "error" => {
            let error =
                font_face_slot_value(scope, face, FONT_FACE_ERROR_SLOT).unwrap_or_else(|| {
                    crate::context_bootstrap::new_dom_exception_value(
                        scope,
                        "The FontFace failed to load.",
                        "NetworkError",
                    )
                });
            let _ = resolver.reject(scope, error);
        }
        _ => set_font_face_slot_value(scope, face, FONT_FACE_LOADED_RESOLVER_SLOT, resolver.into()),
    }
    Some(loaded)
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

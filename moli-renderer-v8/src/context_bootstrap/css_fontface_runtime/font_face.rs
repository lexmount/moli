pub(super) use super::font_loading::start_font_face_load;
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
    #[webapi(slot = FONT_FACE_LOADED_RESOLVER_SLOT)]
    loaded_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    #[webapi(slot = FONT_FACE_ERROR_SLOT)]
    error: Option<v8::Local<'s, v8::Value>>,
    #[webapi(slot = FONT_FACE_SET_OWNERS_SLOT, constructor_default = Vec::new())]
    owner_sets: Vec<v8::Local<'s, v8::Value>>,
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
        getter = font_face_loaded_getter_callback,
        returns_promise,
        enumerable
    )]
    loaded: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FontFace")]
struct FontFaceConstructorArgs<'s> {
    #[webidl(required)]
    family: String,
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
    let (source, status, error, data) = if invalid_descriptors {
        let source = match parsed.source {
            FontFaceConstructorSource::Css(source) => source,
            FontFaceConstructorSource::Binary(_) => String::new(),
        };
        let error = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "Invalid FontFace descriptor.",
            "SyntaxError",
        );
        (source, "error", Some(error), None)
    } else {
        match parsed.source {
            FontFaceConstructorSource::Css(source)
                if moli_css_parse::normalize_font_face_src(&source).is_some() =>
            {
                (source, "unloaded", None, None)
            }
            FontFaceConstructorSource::Css(source) => {
                let error = crate::context_bootstrap::new_dom_exception_value(
                    scope,
                    "Invalid FontFace source descriptor.",
                    "SyntaxError",
                );
                (source, "error", Some(error), None)
            }
            FontFaceConstructorSource::Binary(value) => {
                // BufferSource conversion retains the native buffer; copying its
                // bytes belongs to the operation, after descriptor conversion.
                // A descriptor getter may have modified or detached the buffer.
                let bytes = match webidl::convert::<webidl::BufferSource>(
                    scope,
                    value,
                    webidl::Context::argument("FontFace", 2),
                ) {
                    Ok(bytes) => bytes.into_bytes(),
                    Err(error) => {
                        webidl::throw_error(scope, &error);
                        return;
                    }
                };
                if moli_layout::validate_web_font_bytes(&bytes).is_ok() {
                    (String::new(), "loaded", None, Some(bytes))
                } else {
                    let error = crate::context_bootstrap::new_dom_exception_value(
                        scope,
                        "Invalid font data in ArrayBuffer.",
                        "SyntaxError",
                    );
                    (String::new(), "error", Some(error), None)
                }
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
        None,
        None,
        error,
    )
    .initialize(scope, this)
    .expect("FontFace declaration should initialize object");
    if let Some(data) = data {
        super::font_loading::store_font_face_binary_data(scope, this, data);
    } else {
        super::font_loading::capture_font_face_sources(scope, this);
    }
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
    // This BufferSource union has neither AllowShared nor AllowResizable.
    // Reject those native buffer inputs before reading the descriptors or
    // attempting the CSSOMString alternative of the union.
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

pub(super) fn font_face_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<String> {
    font_face_string_slot(scope, face, FONT_FACE_STATUS_SLOT)
}

pub(super) fn font_face_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    font_face_slot_value(scope, face, slot)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn set_font_face_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    status: &'static str,
) {
    let status = v8_string(scope, status).unwrap_or_else(|| v8::String::empty(scope));
    set_font_face_slot_value(scope, face, FONT_FACE_STATUS_SLOT, status.into());
}

pub(super) fn font_face_loaded_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    font_face_slot_value(scope, face, FONT_FACE_LOADED_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

pub(super) fn ensure_font_face_loaded_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let context = face.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
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

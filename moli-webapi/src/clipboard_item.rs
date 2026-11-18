//! ClipboardItem values and Blob conversion. The embedder manages clipboard access.

use crate::media_capabilities::promises::{set_rejected_promise, try_parse_promise_args};
use crate::{blob, interfaces as web_api_interfaces};
use moli_v8_util::{
    buffer_source_bytes_from_value, get_private_value, throw_type_error, v8_string,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

use ClipboardPresentationStyle as PresentationStyle;

#[derive(Clone, Copy, Debug, Default, moli_webidl::WebIdlEnum)]
#[webidl(name = "PresentationStyle", rename_all = "kebab-case")]
pub enum ClipboardPresentationStyle {
    #[default]
    Unspecified,
    Inline,
    Attachment,
}

#[derive(Clone, Debug, Default)]
pub struct ClipboardSnapshot {
    pub presentation_style: ClipboardPresentationStyle,
    pub representations: Vec<(String, Vec<u8>)>,
}

const CLIPBOARD_ITEM_DATA_SLOT: &str = "__moliClipboardItemData";
const CLIPBOARD_ITEM_RAW_DATA_SLOT: &str = "__moliClipboardItemRawData";
const CLIPBOARD_ITEM_TYPES_SLOT: &str = "__moliClipboardItemTypes";
const CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT: &str = "__moliClipboardItemPresentationStyle";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ClipboardItem)]
struct ClipboardItemObjectDeclaration<'scope> {
    #[webapi(slot = CLIPBOARD_ITEM_DATA_SLOT)]
    data: v8::Local<'scope, v8::Object>,

    #[webapi(slot = CLIPBOARD_ITEM_RAW_DATA_SLOT)]
    raw_data: v8::Local<'scope, v8::Object>,

    #[webapi(slot = CLIPBOARD_ITEM_TYPES_SLOT)]
    types: v8::Local<'scope, v8::Array>,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardItem, enumerable)]
struct ClipboardItemPrototypeDeclaration {
    #[webapi(accessor_property, getter = clipboard_item_presentation_style_getter)]
    presentation_style: (),

    #[webapi(accessor_property, getter = clipboard_item_types_getter)]
    types: (),

    #[webapi(method, length = 1, callback = clipboard_item_get_type_callback, returns_promise)]
    get_type: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardItem, enumerable)]
struct ClipboardItemConstructorDeclaration {
    #[webapi(static_method, length = 1, callback = clipboard_item_supports_callback)]
    supports: (),
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardItemOptions", snapshot)]
struct ClipboardItemOptions {
    #[webidl(converter = "enum", default = PresentationStyle::Unspecified)]
    #[webidl(snapshot_slot = CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT)]
    presentation_style: PresentationStyle,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ClipboardItem")]
struct ClipboardItemConstructorArgs<'scope> {
    #[webidl(required, converter = "raw")]
    items: webidl::Record<webidl::DomString, v8::Local<'scope, v8::Promise>>,

    #[webidl(dictionary)]
    options: ClipboardItemOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ClipboardItem.getType")]
struct ClipboardItemGetTypeArgs {
    #[webidl(required, name = "type")]
    mime_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ClipboardItem.supports")]
struct ClipboardItemSupportsArgs {
    #[webidl(required, name = "type")]
    mime_type: String,
}

pub fn install_clipboard_item_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    ClipboardItemPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
    ClipboardItemConstructorDeclaration::initialize_template(scope, template);
}

pub fn clipboard_item_types<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, item, CLIPBOARD_ITEM_TYPES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub fn clipboard_item_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ClipboardItem': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ClipboardItemConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    if parsed.items.0.is_empty() {
        throw_type_error(
            scope,
            "Failed to construct 'ClipboardItem': The items record must not be empty.",
        );
        return;
    }
    if !initialize_clipboard_item(
        scope,
        args.this(),
        parsed
            .items
            .0
            .into_iter()
            .map(|(mime_type, value)| (mime_type.0, value))
            .collect(),
        parsed.options.presentation_style,
    ) {
        throw_type_error(scope, "Failed to initialize ClipboardItem data.");
        return;
    }
    rv.set(args.this().into());
}

fn initialize_clipboard_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    entries: Vec<(String, v8::Local<'s, v8::Promise>)>,
    presentation_style: PresentationStyle,
) -> bool {
    let data = v8::Object::new(scope);
    let raw_data = v8::Object::new(scope);
    let types = v8::Array::new(scope, entries.len() as i32);
    for (index, (mime_type, promise)) in entries.into_iter().enumerate() {
        let Some(key) = v8_string(scope, &mime_type) else {
            return false;
        };
        let Some(converter) = v8::Function::builder(clipboard_item_data_to_blob_callback)
            .data(key.into())
            .build(scope)
        else {
            return false;
        };
        let Some(normalized) = promise.then(scope, converter) else {
            return false;
        };
        if raw_data.create_data_property(scope, key.into(), promise.into()) != Some(true)
            || data.create_data_property(scope, key.into(), normalized.into()) != Some(true)
            || types.set_index(scope, index as u32, key.into()) != Some(true)
        {
            return false;
        }
    }
    let _ = types.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    ClipboardItemOptions { presentation_style }
        .initialize_snapshot(scope, object)
        .is_some()
        && ClipboardItemObjectDeclaration::new(data, raw_data, types)
            .initialize(scope, object)
            .is_ok()
}

pub fn build_clipboard_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: Vec<(String, v8::Local<'s, v8::Promise>)>,
    presentation_style: PresentationStyle,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = ClipboardItemObjectDeclaration::new(
        v8::Object::new(scope),
        v8::Object::new(scope),
        v8::Array::new(scope, 0),
    )
    .bind(scope)
    .ok()?;
    initialize_clipboard_item(scope, object, entries, presentation_style).then_some(object)
}

fn clipboard_item_data_to_blob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && blob::is_blob_object(scope, object)
    {
        rv.set(value);
        return;
    }
    let Some(text) = value.to_string(scope) else {
        return;
    };
    let mime_type = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let bytes = text.to_rust_string_lossy(scope).into_bytes();
    if let Some(blob) = blob::build_blob_object(scope, bytes, mime_type) {
        rv.set(blob.into());
    }
}

fn clipboard_item_presentation_style_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) =
        get_private_value(scope, args.this(), CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT)
    {
        rv.set(value);
    }
}

fn clipboard_item_types_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), CLIPBOARD_ITEM_TYPES_SLOT) {
        rv.set(value);
    }
}

fn clipboard_item_get_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let parsed = match try_parse_promise_args::<ClipboardItemGetTypeArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let promise = clipboard_item_data_promise(scope, args.this(), &parsed.mime_type);
    if let Some(promise) = promise {
        rv.set(promise.into());
        return;
    }
    let reason = crate::dom_exception::new_dom_exception_value(
        scope,
        "The requested clipboard type was not found.",
        "NotFoundError",
    );
    set_rejected_promise(scope, &mut rv, reason);
}

fn clipboard_item_supports_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ClipboardItemSupportsArgs>(scope, &args) else {
        return;
    };
    rv.set(v8::Boolean::new(scope, clipboard_item_type_supported(&parsed.mime_type)).into());
}

pub fn clipboard_item_type_supported(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "text/plain" | "text/html" | "image/png" | "text/uri-list" | "image/svg+xml"
    ) || mime_type.strip_prefix("web ").is_some_and(valid_mime_type)
}

fn valid_mime_type(mime_type: &str) -> bool {
    if mime_type.contains(';') {
        return false;
    }
    let mut parts = mime_type.split('/');
    let Some(top_level) = parts.next() else {
        return false;
    };
    let Some(subtype) = parts.next() else {
        return false;
    };
    parts.next().is_none() && mime_token(top_level) && mime_token(subtype)
}

fn mime_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

pub fn clipboard_item_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    values: v8::Local<'s, v8::Array>,
) -> Option<ClipboardSnapshot> {
    let style = get_private_value(scope, item, CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT)?;
    let style = v8::Local::<v8::String>::try_from(style)
        .ok()?
        .to_rust_string_lossy(scope);
    let presentation_style = <PresentationStyle as webidl::WebIdlEnum>::parse_token(&style)?;
    let types = clipboard_item_types(scope, item)?;
    if values.length() != types.length() {
        return None;
    }
    let mut representations = Vec::with_capacity(types.length() as usize);
    for index in 0..types.length() {
        let mime_type = types
            .get_index(scope, index)?
            .to_string(scope)?
            .to_rust_string_lossy(scope);
        let bytes = values.get_index(scope, index)?;
        representations.push((mime_type, buffer_source_bytes_from_value(scope, bytes)?));
    }
    Some(ClipboardSnapshot {
        presentation_style,
        representations,
    })
}

pub fn clipboard_item_data_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    mime_type: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let data = get_private_value(scope, item, CLIPBOARD_ITEM_DATA_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let key = v8_string(scope, mime_type)?;
    data.get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
}

pub fn clipboard_item_raw_data_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    mime_type: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let data = get_private_value(scope, item, CLIPBOARD_ITEM_RAW_DATA_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let key = v8_string(scope, mime_type)?;
    data.get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::{clipboard_item_type_supported, valid_mime_type};

    #[test]
    fn clipboard_item_supports_standard_and_web_custom_types() {
        for mime_type in [
            "text/plain",
            "text/html",
            "image/png",
            "text/uri-list",
            "image/svg+xml",
            "web foo/bar",
            "web text/html",
        ] {
            assert!(clipboard_item_type_supported(mime_type), "{mime_type}");
        }
        for mime_type in [
            "web ",
            "web",
            "web foo",
            "foo/bar",
            "weB text/html",
            " web text/html",
            "not a/real type",
            "",
            " ",
        ] {
            assert!(!clipboard_item_type_supported(mime_type), "{mime_type}");
        }
        assert!(valid_mime_type("application/x.example+json"));
        assert!(!valid_mime_type("application//json"));
    }
}

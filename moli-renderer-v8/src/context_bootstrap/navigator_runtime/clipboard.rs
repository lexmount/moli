use super::super::{SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT};
use crate::{
    blob,
    util::{get_private_value, set_private_value, throw_type_error, v8_string},
    webidl,
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const CLIPBOARD_ITEMS_SLOT: &str = "__moliClipboardItems";
const CLIPBOARD_TEXT_SLOT: &str = "__moliClipboardText";
const CLIPBOARD_EVENT_LISTENERS_SLOT: &str = "__moliClipboardEventListeners";

const CLIPBOARD_ITEM_DATA_SLOT: &str = "__moliClipboardItemData";
const CLIPBOARD_ITEM_RAW_DATA_SLOT: &str = "__moliClipboardItemRawData";
const CLIPBOARD_ITEM_TYPES_SLOT: &str = "__moliClipboardItemTypes";
const CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT: &str = "__moliClipboardItemPresentationStyle";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Clipboard")]
struct ClipboardObjectDeclaration {
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = CLIPBOARD_EVENT_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = CLIPBOARD_ITEMS_SLOT, init = "undefined")]
    items: (),

    #[webapi(slot = CLIPBOARD_TEXT_SLOT, init = "")]
    text: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Clipboard", enumerable)]
struct ClipboardPrototypeDeclaration {
    #[webapi(method, length = 0, callback = clipboard_read_callback)]
    read: (),

    #[webapi(method, length = 0, callback = clipboard_read_text_callback)]
    read_text: (),

    #[webapi(method, length = 1, callback = clipboard_write_callback)]
    write: (),

    #[webapi(method, length = 1, callback = clipboard_write_text_callback)]
    write_text: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "ClipboardItem")]
struct ClipboardItemObjectDeclaration<'scope> {
    #[webapi(slot = CLIPBOARD_ITEM_DATA_SLOT)]
    data: v8::Local<'scope, v8::Object>,

    #[webapi(slot = CLIPBOARD_ITEM_RAW_DATA_SLOT)]
    raw_data: v8::Local<'scope, v8::Object>,

    #[webapi(slot = CLIPBOARD_ITEM_TYPES_SLOT)]
    types: v8::Local<'scope, v8::Array>,

    #[webapi(slot = CLIPBOARD_ITEM_PRESENTATION_STYLE_SLOT)]
    presentation_style: String,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "ClipboardItem", enumerable)]
struct ClipboardItemPrototypeDeclaration {
    #[webapi(accessor_property, getter = clipboard_item_presentation_style_getter)]
    presentation_style: (),

    #[webapi(accessor_property, getter = clipboard_item_types_getter)]
    types: (),

    #[webapi(method, length = 1, callback = clipboard_item_get_type_callback)]
    get_type: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "ClipboardItem", enumerable)]
struct ClipboardItemConstructorDeclaration {
    #[webapi(static_method, length = 1, callback = clipboard_item_supports_callback)]
    supports: (),
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "PresentationStyle", rename_all = "kebab-case")]
enum PresentationStyle {
    #[default]
    Unspecified,
    Inline,
    Attachment,
}

impl PresentationStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Inline => "inline",
            Self::Attachment => "attachment",
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardItemOptions")]
struct ClipboardItemOptions {
    #[webidl(converter = "enum", default = PresentationStyle::Unspecified)]
    presentation_style: PresentationStyle,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardUnsanitizedFormats")]
struct ClipboardUnsanitizedFormats {
    #[webidl(with = clipboard_unsanitized_formats_member)]
    unsanitized: Option<Vec<String>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ClipboardItem")]
struct ClipboardItemConstructorArgs<'scope> {
    #[webidl(required, with = clipboard_item_record_arg)]
    items: Vec<(String, v8::Local<'scope, v8::Promise>)>,

    #[webidl(with = clipboard_item_options_arg)]
    options: ClipboardItemOptions,
}

struct ClipboardItemReference<'scope>(v8::Local<'scope, v8::Object>);

impl<'scope> webidl::WebIdlConverter<'scope> for ClipboardItemReference<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> std::result::Result<Self, webidl::WebIdlError> {
        let object = webidl::convert::<v8::Local<'scope, v8::Object>>(scope, value, context)?;
        if !clipboard_item_receiver_branded(scope, object) {
            return Err(webidl::WebIdlError::custom_message(
                "Clipboard.write data must contain ClipboardItem objects.",
            ));
        }
        Ok(Self(object))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.read")]
struct ClipboardReadArgs {
    #[webidl(with = clipboard_read_formats_arg)]
    formats: ClipboardUnsanitizedFormats,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.write")]
struct ClipboardWriteArgs<'scope> {
    #[webidl(required, with = clipboard_write_data_arg)]
    data: Vec<v8::Local<'scope, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.writeText")]
struct ClipboardWriteTextArgs {
    #[webidl(required)]
    data: String,
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

pub(super) fn install_clipboard_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "Clipboard" => ClipboardPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        ),
        "ClipboardItem" => {
            ClipboardItemPrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
            ClipboardItemConstructorDeclaration::initialize_template(scope, template);
        }
        _ => {}
    }
}

pub(super) fn build_clipboard_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    ClipboardObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind navigator.clipboard object: {error}"))
}

pub(in crate::context_bootstrap) fn clipboard_item_constructor_callback<'s>(
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
    if parsed.items.is_empty() {
        throw_type_error(
            scope,
            "Failed to construct 'ClipboardItem': The items record must not be empty.",
        );
        return;
    }
    if !initialize_clipboard_item(
        scope,
        args.this(),
        parsed.items,
        parsed.options.presentation_style,
    ) {
        throw_type_error(scope, "Failed to initialize ClipboardItem data.");
        return;
    }
    rv.set(args.this().into());
}

fn clipboard_item_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<ClipboardItemOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("ClipboardItem", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn clipboard_item_record_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<Vec<(String, v8::Local<'s, v8::Promise>)>, webidl::WebIdlError> {
    let context = webidl::Context::argument("ClipboardItem", (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::Record<webidl::DomString, v8::Local<'s, v8::Promise>>>(
        scope,
        args.get(index),
        context,
    )
    .map(|record| {
        record
            .0
            .into_iter()
            .map(|(mime_type, value)| (mime_type.0, value))
            .collect()
    })
}

fn clipboard_unsanitized_formats_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<Vec<String>>, webidl::WebIdlError> {
    let context = webidl::Context::member("ClipboardUnsanitizedFormats", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    webidl::convert::<webidl::Sequence<webidl::DomString>>(scope, value, context)
        .map(|sequence| Some(sequence.0.into_iter().map(|format| format.0).collect()))
}

fn clipboard_write_data_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<Vec<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let context = webidl::Context::argument("Clipboard.write", (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::Sequence<ClipboardItemReference<'s>>>(scope, args.get(index), context)
        .map(|sequence| sequence.0.into_iter().map(|item| item.0).collect())
}

fn clipboard_read_formats_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<ClipboardUnsanitizedFormats, webidl::WebIdlError> {
    let context = webidl::Context::argument("Clipboard.read", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|formats| formats.unwrap_or_default())
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
    ClipboardItemObjectDeclaration::new(
        data,
        raw_data,
        types,
        presentation_style.as_str().to_owned(),
    )
    .initialize(scope, object)
    .is_ok()
}

fn build_clipboard_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: Vec<(String, v8::Local<'s, v8::Promise>)>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = ClipboardItemObjectDeclaration::new(
        v8::Object::new(scope),
        v8::Object::new(scope),
        v8::Array::new(scope, 0),
        PresentationStyle::Unspecified.as_str().to_owned(),
    )
    .bind(scope)
    .ok()?;
    initialize_clipboard_item(scope, object, entries, PresentationStyle::Unspecified)
        .then_some(object)
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

fn clipboard_item_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, receiver, "ClipboardItem")
}

fn clipboard_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, receiver, "Clipboard")
}

fn clipboard_item_presentation_style_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_item_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
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
    if !clipboard_item_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(value) = get_private_value(scope, args.this(), CLIPBOARD_ITEM_TYPES_SLOT) {
        rv.set(value);
    }
}

fn clipboard_item_get_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_item_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let parsed = match try_parse_promise_args::<ClipboardItemGetTypeArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let promise = get_private_value(scope, args.this(), CLIPBOARD_ITEM_DATA_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|data| {
            let key = v8_string(scope, &parsed.mime_type)?;
            data.get(scope, key.into())
        })
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    if let Some(promise) = promise {
        rv.set(promise.into());
        return;
    }
    let reason = super::super::new_dom_exception_value(
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

fn clipboard_item_type_supported(mime_type: &str) -> bool {
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

fn clipboard_read_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let parsed = match try_parse_promise_args::<ClipboardReadArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    if let Some(formats) = parsed.formats.unsanitized {
        let invalid = formats.len() > 1 || formats.iter().any(|format| format != "text/html");
        if invalid {
            let reason = super::super::new_dom_exception_value(
                scope,
                "The requested unsanitized clipboard format is not allowed.",
                "NotAllowedError",
            );
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    }
    let items = copy_clipboard_items(scope, args.this());
    set_resolved_promise(scope, &mut rv, items.into());
}

fn clipboard_read_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    if let Some(text) =
        get_private_value(scope, args.this(), CLIPBOARD_TEXT_SLOT).filter(|value| value.is_string())
    {
        set_resolved_promise(scope, &mut rv, text);
        return;
    }
    let text_promise = first_clipboard_item(scope, args.this())
        .and_then(|item| clipboard_item_data_promise(scope, item, "text/plain"))
        .and_then(|promise| {
            let converter = v8::Function::builder(clipboard_blob_to_text_callback).build(scope)?;
            promise.then(scope, converter)
        });
    if let Some(promise) = text_promise {
        rv.set(promise.into());
    } else {
        let empty = v8::String::empty(scope);
        set_resolved_promise(scope, &mut rv, empty.into());
    }
}

fn clipboard_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let parsed = match try_parse_promise_args::<ClipboardWriteArgs<'s>>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    if parsed.data.len() != 1 {
        let reason = super::super::new_dom_exception_value(
            scope,
            "Only one ClipboardItem can be written at a time.",
            "NotAllowedError",
        );
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let item = parsed.data[0];
    let Some(validation) = clipboard_item_write_validation_promise(scope, item) else {
        let reason = type_error_value(scope, "Failed to validate ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    let callback_data = v8::Array::new(scope, 2);
    let _ = callback_data.set_index(scope, 0, args.this().into());
    let _ = callback_data.set_index(scope, 1, item.into());
    let Some(store_callback) = v8::Function::builder(clipboard_store_validated_item_callback)
        .data(callback_data.into())
        .build(scope)
    else {
        let reason = type_error_value(scope, "Failed to write ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    let Some(promise) = validation.then(scope, store_callback) else {
        let reason = type_error_value(scope, "Failed to write ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    rv.set(promise.into());
}

fn clipboard_write_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !clipboard_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let parsed = match try_parse_promise_args::<ClipboardWriteTextArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let Some(text) = v8_string(scope, &parsed.data) else {
        rv.set_undefined();
        return;
    };
    let Some(promise) = resolved_promise(scope, text.into()) else {
        rv.set_undefined();
        return;
    };
    let Some(item) = build_clipboard_item(scope, vec![("text/plain".to_owned(), promise)]) else {
        rv.set_undefined();
        return;
    };
    store_clipboard_item(scope, args.this(), item);
    set_private_value(scope, args.this(), CLIPBOARD_TEXT_SLOT, text.into());
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn clipboard_blob_to_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(blob) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, "Clipboard text data is not a Blob.");
        return;
    };
    let Some(bytes) = blob::blob_bytes_from_object(scope, blob) else {
        throw_type_error(scope, "Clipboard text data is not a Blob.");
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    if let Some(value) = v8_string(scope, &text) {
        rv.set(value.into());
    }
}

fn clipboard_item_write_validation_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let types = get_private_value(scope, item, CLIPBOARD_ITEM_TYPES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    let mut custom_format_count = 0_u32;
    let mut entries = Vec::with_capacity(types.length() as usize);
    for index in 0..types.length() {
        let key = types.get_index(scope, index)?;
        let mime_type = key.to_string(scope)?.to_rust_string_lossy(scope);
        if mime_type.starts_with("web ") {
            custom_format_count += 1;
        }
        let raw = clipboard_item_raw_data_promise(scope, item, &mime_type)?;
        entries.push((key, raw));
    }
    if custom_format_count > 100 {
        let reason = super::super::new_dom_exception_value(
            scope,
            "The ClipboardItem contains too many custom formats.",
            "NotAllowedError",
        );
        return rejected_promise(scope, reason);
    }
    if entries.is_empty() {
        return resolved_promise(scope, v8::undefined(scope).into());
    }

    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let state = v8::Array::new(scope, 3);
    let _ = state.set_index(scope, 0, resolver.into());
    let _ = state.set_index(
        scope,
        1,
        v8::Integer::new_from_unsigned(scope, entries.len() as u32).into(),
    );
    let _ = state.set_index(scope, 2, v8::Boolean::new(scope, false).into());

    for (key, raw) in entries {
        let validator = v8::Function::builder(clipboard_validate_write_entry_callback)
            .data(key)
            .build(scope)?;
        let validated = raw.then(scope, validator)?;
        let on_fulfilled = v8::Function::builder(clipboard_validation_fulfilled_callback)
            .data(state.into())
            .build(scope)?;
        let on_rejected = v8::Function::builder(clipboard_validation_rejected_callback)
            .data(state.into())
            .build(scope)?;
        validated.then2(scope, on_fulfilled, on_rejected)?;
    }
    Some(promise)
}

fn clipboard_validate_write_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let mime_type = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if !clipboard_item_type_supported(&mime_type) {
        throw_dom_exception(
            scope,
            "The ClipboardItem contains an unsupported representation type.",
            "NotAllowedError",
        );
        return;
    }
    let value = args.get(0);
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        if mime_type == "image/png" {
            throw_type_error(
                scope,
                "The image/png clipboard representation must be a Blob.",
            );
        }
        return;
    };
    if !blob::is_blob_object(scope, object) {
        if mime_type == "image/png" {
            throw_type_error(
                scope,
                "The image/png clipboard representation must be a Blob.",
            );
        }
        return;
    }
    if blob::blob_mime_type_from_object(scope, object).as_deref() != Some(mime_type.as_str()) {
        throw_dom_exception(
            scope,
            "The ClipboardItem representation type does not match its Blob type.",
            "NotAllowedError",
        );
    }
}

fn clipboard_validation_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, resolver)) = clipboard_validation_state(scope, args.data()) else {
        return;
    };
    if state
        .get_index(scope, 2)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return;
    }
    let remaining = state
        .get_index(scope, 1)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(1);
    if remaining <= 1 {
        let _ = state.set_index(scope, 2, v8::Boolean::new(scope, true).into());
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    } else {
        let _ = state.set_index(
            scope,
            1,
            v8::Integer::new_from_unsigned(scope, remaining - 1).into(),
        );
    }
}

fn clipboard_validation_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((state, resolver)) = clipboard_validation_state(scope, args.data()) else {
        return;
    };
    if state
        .get_index(scope, 2)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return;
    }
    let _ = state.set_index(scope, 2, v8::Boolean::new(scope, true).into());
    let _ = resolver.reject(scope, args.get(0));
}

fn clipboard_validation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Array>, v8::Local<'s, v8::PromiseResolver>)> {
    let state = v8::Local::<v8::Array>::try_from(value).ok()?;
    let resolver = state
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })?;
    Some((state, resolver))
}

fn clipboard_store_validated_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        throw_type_error(scope, "Failed to write ClipboardItem data.");
        return;
    };
    let clipboard = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let item = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let (Some(clipboard), Some(item)) = (clipboard, item) else {
        throw_type_error(scope, "Failed to write ClipboardItem data.");
        return;
    };
    store_clipboard_item(scope, clipboard, item);
    set_private_value(
        scope,
        clipboard,
        CLIPBOARD_TEXT_SLOT,
        v8::undefined(scope).into(),
    );
}

fn throw_dom_exception(scope: &mut v8::PinScope<'_, '_>, message: &str, name: &str) {
    let exception = super::super::new_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}

fn store_clipboard_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
    item: v8::Local<'s, v8::Object>,
) {
    let items = v8::Array::new(scope, 1);
    let _ = items.set_index(scope, 0, item.into());
    let _ = items.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    set_private_value(scope, clipboard, CLIPBOARD_ITEMS_SLOT, items.into());
}

fn copy_clipboard_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let item = first_clipboard_item(scope, clipboard);
    let result = v8::Array::new(scope, i32::from(item.is_some()));
    if let Some(item) = item {
        let _ = result.set_index(scope, 0, item.into());
    }
    result
}

fn first_clipboard_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, clipboard, CLIPBOARD_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .and_then(|items| items.get_index(scope, 0))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn clipboard_item_data_promise<'s>(
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

fn clipboard_item_raw_data_promise<'s>(
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

fn try_parse_promise_args<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> std::result::Result<T, v8::Local<'s, v8::Value>>
where
    T: webidl::WebIdlArguments<'s>,
{
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match webidl::try_parse_args::<T>(&mut scope, args) {
        Ok(parsed) => Ok(parsed),
        Err(error) if error.is_pending_exception() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        Err(error) => Err(type_error_value(&mut scope, &error.to_string())),
    }
}

fn type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    v8::Exception::type_error(scope, message)
}

fn resolved_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    (resolver.resolve(scope, value) == Some(true)).then_some(promise)
}

fn rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    (resolver.reject(scope, reason) == Some(true)).then_some(promise)
}

fn set_resolved_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(promise) = resolved_promise(scope, value) {
        rv.set(promise.into());
    } else {
        rv.set_undefined();
    }
}

fn set_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
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

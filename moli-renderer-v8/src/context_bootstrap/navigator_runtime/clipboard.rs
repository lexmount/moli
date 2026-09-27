use super::super::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
use crate::web_api_interfaces;
use crate::{
    blob,
    runtime::{ClipboardPresentationStyle as PresentationStyle, ClipboardSnapshot},
    util::{context_host_ptr_from_context_slot, throw_type_error, v8_string},
    webidl,
};
use anyhow::{Result, anyhow};
pub(in crate::context_bootstrap) use moli_webapi::clipboard_item::clipboard_item_constructor_callback;
use moli_webapi::clipboard_item::{
    build_clipboard_item, clipboard_item_data_promise, clipboard_item_raw_data_promise,
    clipboard_item_snapshot, clipboard_item_type_supported, clipboard_item_types,
    install_clipboard_item_template_bindings,
};
use moli_webapi::navigator_options::ClipboardUnsanitizedFormats;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::Clipboard)]
struct ClipboardObjectDeclaration {
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Clipboard, enumerable)]
struct ClipboardPrototypeDeclaration {
    #[webapi(method, length = 0, callback = clipboard_read_callback, returns_promise)]
    read: (),

    #[webapi(method, length = 0, callback = clipboard_read_text_callback, returns_promise)]
    read_text: (),

    #[webapi(method, length = 1, callback = clipboard_write_callback, returns_promise)]
    write: (),

    #[webapi(method, length = 1, callback = clipboard_write_text_callback, returns_promise)]
    write_text: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.read")]
struct ClipboardReadArgs {
    #[webidl(dictionary)]
    formats: ClipboardUnsanitizedFormats,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.write")]
struct ClipboardWriteArgs<'scope> {
    #[webidl(required, sequence, interface = web_api_interfaces::ClipboardItem)]
    data: Vec<v8::Local<'scope, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Clipboard.writeText")]
struct ClipboardWriteTextArgs {
    #[webidl(required)]
    data: String,
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
        "ClipboardItem" => install_clipboard_item_template_bindings(scope, template),
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

fn clipboard_read_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let parsed = match try_parse_promise_args::<ClipboardReadArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let Some(runtime) = clipboard_browser_context(scope, args.this()) else {
        return;
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
    if reject_denied_clipboard_access(scope, args.this(), "clipboard-read", &mut rv) {
        return;
    }
    let Some(items) = copy_clipboard_items(scope, &runtime) else {
        let reason = type_error_value(scope, "Failed to read clipboard data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    set_resolved_promise(scope, &mut rv, items.into());
}

fn clipboard_read_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(runtime) = clipboard_browser_context(scope, args.this()) else {
        return;
    };
    if reject_denied_clipboard_access(scope, args.this(), "clipboard-read", &mut rv) {
        return;
    }
    let bytes = runtime
        .clipboard_data_for_type("text/plain")
        .unwrap_or_default();
    if let Some(text) = v8_string(scope, &String::from_utf8_lossy(&bytes)) {
        set_resolved_promise(scope, &mut rv, text.into());
    }
}

fn clipboard_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let parsed = match try_parse_promise_args::<ClipboardWriteArgs<'s>>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    if clipboard_browser_context(scope, args.this()).is_none() {
        return;
    }
    if parsed.data.len() != 1 {
        let reason = super::super::new_dom_exception_value(
            scope,
            "Only one ClipboardItem can be written at a time.",
            "NotAllowedError",
        );
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    if reject_denied_clipboard_access(scope, args.this(), "clipboard-write", &mut rv) {
        return;
    }
    let item = parsed.data[0];
    let Some(validation) = clipboard_item_write_validation_promise(scope, item) else {
        let reason = type_error_value(scope, "Failed to validate ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let callback_data = v8::Array::new(scope, 3);
    let _ = callback_data.set_index(scope, 0, args.this().into());
    let _ = callback_data.set_index(scope, 1, item.into());
    let _ = callback_data.set_index(scope, 2, resolver.into());
    let Some(store_callback) = v8::Function::builder(clipboard_store_validated_item_callback)
        .data(callback_data.into())
        .build(scope)
    else {
        let reason = type_error_value(scope, "Failed to write ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    let Some(reject_callback) = v8::Function::builder(clipboard_write_rejected_callback)
        .data(callback_data.into())
        .build(scope)
    else {
        let reason = type_error_value(scope, "Failed to write ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    if validation
        .then2(scope, store_callback, reject_callback)
        .is_none()
    {
        let reason = type_error_value(scope, "Failed to write ClipboardItem data.");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    rv.set(resolver.get_promise(scope).into());
}

fn clipboard_write_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let parsed = match try_parse_promise_args::<ClipboardWriteTextArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let Some(runtime) = clipboard_browser_context(scope, args.this()) else {
        return;
    };
    if reject_denied_clipboard_access(scope, args.this(), "clipboard-write", &mut rv) {
        return;
    }
    runtime.set_clipboard_snapshot(ClipboardSnapshot {
        presentation_style: PresentationStyle::Unspecified,
        representations: vec![("text/plain".to_owned(), parsed.data.into_bytes())],
    });
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn clipboard_item_write_validation_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let types = clipboard_item_types(scope, item)?;
    let mut custom_format_count = 0_u32;
    let mut entries = Vec::with_capacity(types.length() as usize);
    for index in 0..types.length() {
        let key = types.get_index(scope, index)?;
        let mime_type = key.to_string(scope)?.to_rust_string_lossy(scope);
        if mime_type.starts_with("web ") {
            custom_format_count += 1;
        }
        let raw = clipboard_item_raw_data_promise(scope, item, &mime_type)?;
        let normalized = clipboard_item_data_promise(scope, item, &mime_type)?;
        let validation_data = v8::Array::new(scope, 2);
        let _ = validation_data.set_index(scope, 0, key);
        let _ = validation_data.set_index(scope, 1, normalized.into());
        entries.push((validation_data, raw));
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
        let values = v8::Array::new(scope, 0);
        return resolved_promise(scope, values.into());
    }

    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let state = v8::Array::new(scope, 4);
    let _ = state.set_index(scope, 0, resolver.into());
    let _ = state.set_index(
        scope,
        1,
        v8::Integer::new_from_unsigned(scope, entries.len() as u32).into(),
    );
    let _ = state.set_index(scope, 2, v8::Boolean::new(scope, false).into());
    let values = v8::Array::new(scope, 0);
    let _ = state.set_index(scope, 3, values.into());

    for (index, (validation_data, raw)) in entries.into_iter().enumerate() {
        let validator = v8::Function::builder(clipboard_validate_write_entry_callback)
            .data(validation_data.into())
            .build(scope)?;
        let validated = raw.then(scope, validator)?;
        let reader = v8::Function::new(scope, clipboard_read_write_entry_callback)?;
        let bytes = validated.then(scope, reader)?;
        let fulfilled_data = v8::Array::new(scope, 2);
        let _ = fulfilled_data.set_index(scope, 0, state.into());
        let _ = fulfilled_data.set_index(
            scope,
            1,
            v8::Integer::new_from_unsigned(scope, index as u32).into(),
        );
        let on_fulfilled = v8::Function::builder(clipboard_validation_fulfilled_callback)
            .data(fulfilled_data.into())
            .build(scope)?;
        let on_rejected = v8::Function::builder(clipboard_validation_rejected_callback)
            .data(state.into())
            .build(scope)?;
        bytes.then2(scope, on_fulfilled, on_rejected)?;
    }
    Some(promise)
}

fn clipboard_validate_write_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        throw_type_error(scope, "Failed to validate ClipboardItem data.");
        return;
    };
    let mime_type = data
        .get_index(scope, 0)
        .and_then(|value| value.to_string(scope))
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
    // Await the representation's normalization as well as its raw type
    // validation before committing a byte snapshot to the shared clipboard.
    if let Some(normalized) = data.get_index(scope, 1) {
        rv.set(normalized);
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

fn clipboard_read_write_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|object| blob::blob_data_from_object(scope, object));
    let Some(data) = data else {
        throw_type_error(scope, "Failed to read validated ClipboardItem data.");
        return;
    };
    if let Some(promise) =
        blob::read::read_internal_promise(scope, data, blob::read::BlobReadFormat::Bytes)
        && let Some(rejected) =
            v8::Function::new(scope, clipboard_read_write_entry_rejected_callback)
        && let Some(promise) = promise.catch(scope, rejected)
    {
        rv.set(promise.into());
    }
}

fn clipboard_read_write_entry_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Match Blink's RejectFromReadOrDecodeFailure. This handles only body
    // reads, so author-provided representation Promise errors keep identity.
    throw_dom_exception(scope, "Failed to read ClipboardItem data.", "DataError");
}

fn clipboard_validation_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(state) = data.get_index(scope, 0) else {
        return;
    };
    let Some((state, resolver)) = clipboard_validation_state(scope, state) else {
        return;
    };
    if state
        .get_index(scope, 2)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return;
    }
    let Some(values) = state
        .get_index(scope, 3)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    let Some(index) = data
        .get_index(scope, 1)
        .and_then(|value| value.uint32_value(scope))
        .and_then(|index| v8_string(scope, &index.to_string()))
    else {
        return;
    };
    if values.create_data_property(scope, index.into(), args.get(0)) != Some(true) {
        return;
    }
    let remaining = state
        .get_index(scope, 1)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(1);
    if remaining <= 1 {
        let _ = state.set_index(scope, 2, v8::Boolean::new(scope, true).into());
        let _ = resolver.resolve(scope, values.into());
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
    let Some((clipboard, item, resolver)) = clipboard_write_callback_state(scope, args.data())
    else {
        return;
    };
    // Like Chromium's ScriptPromiseResolver, do not settle a write after its
    // realm is retired. Each live write still commits independently.
    let Some(runtime) = clipboard_browser_context(scope, clipboard) else {
        return;
    };
    if clipboard_permission_denied(scope, clipboard, "clipboard-write") {
        let reason = super::super::new_dom_exception_value(
            scope,
            "Clipboard write permission denied.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let Some(snapshot) = v8::Local::<v8::Array>::try_from(args.get(0))
        .ok()
        .and_then(|values| clipboard_item_snapshot(scope, item, values))
    else {
        let reason = type_error_value(scope, "Failed to read validated ClipboardItem data.");
        let _ = resolver.reject(scope, reason);
        return;
    };
    runtime.set_clipboard_snapshot(snapshot);
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn clipboard_write_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((clipboard, _item, resolver)) = clipboard_write_callback_state(scope, args.data())
    else {
        return;
    };
    if clipboard_browser_context(scope, clipboard).is_some() {
        let _ = resolver.reject(scope, args.get(0));
    }
}

fn clipboard_write_callback_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::PromiseResolver>,
)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let clipboard = v8::Local::<v8::Object>::try_from(data.get_index(scope, 0)?).ok()?;
    let item = v8::Local::<v8::Object>::try_from(data.get_index(scope, 1)?).ok()?;
    let resolver = v8::Local::<v8::Object>::try_from(data.get_index(scope, 2)?).ok()?;
    // This callback data is created only by clipboard_write_callback.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    Some((clipboard, item, resolver))
}

fn throw_dom_exception(scope: &mut v8::PinScope<'_, '_>, message: &str, name: &str) {
    let exception = super::super::new_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}

fn clipboard_permission_denied<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
    permission: &str,
) -> bool {
    let Some(context) = clipboard.get_creation_context(scope) else {
        return true;
    };
    let Some(host_ptr) = context_host_ptr_from_context_slot(context) else {
        return true;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let host = unsafe { &mut *host_ptr };
    let embedding_origin = host.main_document_origin();
    let Some(requesting_origin) = host
        .current_runtime_window_execution_context_identity(scope)
        .and_then(|identity| host.storage_context_for_window_execution_context_identity(identity))
        .map(|context| context.storage_key().serialized_storage_key())
        .and_then(|key| moli_storage_key::deserialize_serialized_storage_key(&key))
        .map(|key| key.origin().to_owned())
    else {
        return true;
    };
    host.permission_state_for_origins(permission, &requesting_origin, &embedding_origin) == "denied"
}

fn reject_denied_clipboard_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
    permission: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> bool {
    if !clipboard_permission_denied(scope, clipboard, permission) {
        return false;
    }
    let reason = super::super::new_dom_exception_value(
        scope,
        "Clipboard permission denied.",
        "NotAllowedError",
    );
    set_rejected_promise(scope, rv, reason);
    true
}

fn clipboard_browser_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    clipboard: v8::Local<'s, v8::Object>,
) -> Option<crate::runtime::RendererBrowserContextRuntime> {
    let context = clipboard.get_creation_context(scope)?;
    let host_ptr = context_host_ptr_from_context_slot(context)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let host = unsafe { &*host_ptr };
    // Blink's CreateForRead/Write entry points return an empty Promise for an
    // invalid ScriptState. Check the receiver realm after WebIDL conversion,
    // which can itself detach the document through author code.
    host.current_runtime_window_execution_context_identity(scope)?;
    Some(host.browser_context_runtime())
}

fn copy_clipboard_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &crate::runtime::RendererBrowserContextRuntime,
) -> Option<v8::Local<'s, v8::Array>> {
    let ClipboardSnapshot {
        presentation_style,
        representations,
    } = runtime.clipboard_snapshot();
    let result = v8::Array::new(scope, i32::from(!representations.is_empty()));
    if !representations.is_empty() {
        let mut entries = Vec::with_capacity(representations.len());
        for (mime_type, bytes) in representations {
            let blob = blob::build_blob_object(scope, bytes, mime_type.clone())?;
            let promise = resolved_promise(scope, blob.into())?;
            entries.push((mime_type, promise));
        }
        let item = build_clipboard_item(scope, entries, presentation_style)?;
        result.set_index(scope, 0, item.into())?;
    }
    Some(result)
}

use super::promises::try_parse_promise_args;

use super::promises::type_error_value;

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

use super::promises::set_rejected_promise;

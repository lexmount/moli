use moli_file_api::{
    BlobId, BlobLineEndings, BlobStore, blob_slice_relative_index, clamp_blob_long_long,
    normalize_blob_line_endings_with_native_ending, normalize_blob_mime_type,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::sync::{Arc, OnceLock};

use super::{
    native_bridge,
    resource_owner::{ResourceOwnerId, current_resource_owner_id},
    runtime::RendererStoragePartitionIdentity,
    util::{get_private_value, set_private_value, throw_type_error, v8_string},
    webidl,
};

const BLOB_ID_SLOT: &str = "__lmBlobId";
const BLOB_PROTOTYPE_SLOT: &str = "__lmBlobPrototype";
const BLOB_REALM_PROTOTYPE_SLOT: &str = "__lmBlobRealmPrototype";

fn native_blob_line_ending() -> &'static str {
    if moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE
        .platform
        .starts_with("Win")
    {
        "\r\n"
    } else {
        "\n"
    }
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct BlobInstanceDeclaration<'scope> {
    #[webapi(slot = BLOB_ID_SLOT)]
    blob_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Blob", enumerable)]
struct BlobPrototypeDeclaration {
    #[webapi(accessor_property, getter = blob_size_attribute_getter_callback)]
    size: (),
    #[webapi(accessor_property, getter = blob_type_attribute_getter_callback)]
    r#type: (),
}

static BLOB_STORE: OnceLock<BlobStore<ResourceOwnerId, RendererStoragePartitionIdentity>> =
    OnceLock::new();

fn blob_store() -> &'static BlobStore<ResourceOwnerId, RendererStoragePartitionIdentity> {
    BLOB_STORE.get_or_init(BlobStore::default)
}

fn current_blob_storage_partition_identity(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<RendererStoragePartitionIdentity> {
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        return Some(
            unsafe { &*host_ptr }
                .browser_context_runtime()
                .storage_partition_identity(),
        );
    }
    crate::worker::worker_storage_partition_identity(scope)
}

pub(super) fn blob_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Blob': Please use the 'new' operator.",
        );
        return;
    }

    let Some((bytes, mime_type)) = collect_blob_bytes_and_type(scope, args.get(0), args.get(1))
    else {
        return;
    };

    init_blob_object(scope, args.this(), bytes, mime_type);
    rv.set(args.this().into());
}

pub(super) fn install_blob_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "Blob" {
        BlobPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(super) fn finalize_blob_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) {
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, BLOB_PROTOTYPE_SLOT, prototype.into());
}

fn blob_size_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let size = blob_bytes_from_object(scope, args.this())
        .map(|bytes| bytes.len() as f64)
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, size).into());
}

fn blob_type_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let mime_type = blob_mime_type_from_object(scope, args.this()).unwrap_or_default();
    if let Some(value) = v8_string(scope, &mime_type) {
        rv.set(value.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

pub(super) fn blob_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        set_rejected_type_error_promise(scope, &mut rv, "Illegal invocation");
        return;
    }
    let text = blob_bytes_from_object(scope, args.this())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &text) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    set_resolved_promise(scope, &mut rv, value.into());
}

pub(super) fn blob_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        set_rejected_type_error_promise(scope, &mut rv, "Illegal invocation");
        return;
    }
    let bytes = blob_bytes_from_object(scope, args.this()).unwrap_or_default();
    let value = array_buffer_from_bytes(scope, bytes)
        .map(|buffer| buffer.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_resolved_promise(scope, &mut rv, value);
}

pub(super) fn blob_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        set_rejected_type_error_promise(scope, &mut rv, "Illegal invocation");
        return;
    }
    let bytes = blob_bytes_from_object(scope, args.this()).unwrap_or_default();
    let value = new_uint8_array_from_bytes(scope, bytes)
        .map(|array| array.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_resolved_promise(scope, &mut rv, value);
}

pub(super) fn blob_stream_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(bytes) = blob_bytes_from_object(scope, args.this()) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let len = bytes.len();
    let Some(buffer) = array_buffer_from_bytes(scope, bytes) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(stream) =
        crate::context_bootstrap::new_readable_stream_from_array_buffer(scope, buffer, len)
    else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    rv.set(stream.into());
}

pub(super) fn blob_slice_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_blob_object(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let bytes = blob_bytes_from_object(scope, args.this()).unwrap_or_default();
    let size = bytes.len();
    let start = clamped_long_long_arg(scope, &args, 0).unwrap_or(0);
    let end = if args.length() > 1 && !args.get(1).is_undefined() {
        clamped_long_long_arg(scope, &args, 1).unwrap_or(size as i64)
    } else {
        size as i64
    };
    let Some(mime_type) = blob_slice_content_type(scope, &args) else {
        return;
    };

    let relative_start = blob_slice_relative_index(start, size);
    let relative_end = blob_slice_relative_index(end, size);
    let span = relative_end.saturating_sub(relative_start);
    let sliced = bytes
        .get(relative_start..relative_start + span)
        .map(ToOwned::to_owned)
        .unwrap_or_default();

    let prototype = blob_realm_prototype_from_object(scope, args.this())
        .or_else(|| current_realm_blob_prototype(scope));
    if let Some(blob) = prototype
        .and_then(|prototype| build_blob_object_with_prototype(scope, sliced, mime_type, prototype))
    {
        rv.set(blob.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn init_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
    mime_type: String,
) -> BlobId {
    let owner_id = current_resource_owner_id(scope);
    let partition_id = current_blob_storage_partition_identity(scope);
    let blob_id = blob_store().create_blob(owner_id, partition_id, bytes, mime_type);
    BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .initialize(scope, object)
        .expect("Blob instance declaration should initialize");
    if let Some(prototype) = current_realm_blob_prototype(scope) {
        set_private_value(scope, object, BLOB_REALM_PROTOTYPE_SLOT, prototype.into());
    }
    track_blob_ref_lifetime(scope, object, move || release_blob_wrapper_ref(blob_id));
    blob_id
}

pub(super) fn build_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = current_realm_blob_prototype(scope)?;
    build_blob_object_with_prototype(scope, bytes, mime_type, prototype)
}

fn current_realm_blob_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, BLOB_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .or_else(|| {
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "Blob")
                    .ok()?;
            finalize_blob_realm_bindings(scope, prototype);
            Some(prototype)
        })
}

fn blob_realm_prototype_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, BLOB_REALM_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn build_blob_object_with_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    mime_type: String,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner_id = current_resource_owner_id(scope);
    let partition_id = current_blob_storage_partition_identity(scope);
    let blob_id = blob_store().create_blob(owner_id, partition_id, bytes, mime_type);
    let object = BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .bind(scope)
        .ok()
        .or_else(|| {
            release_blob_wrapper_ref(blob_id);
            None
        })?;
    let _ = object.set_prototype(scope, prototype.into());
    set_private_value(scope, object, BLOB_REALM_PROTOTYPE_SLOT, prototype.into());
    track_blob_ref_lifetime(scope, object, move || release_blob_wrapper_ref(blob_id));
    Some(object)
}

fn blob_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobId> {
    let value = get_private_value(scope, object, BLOB_ID_SLOT)?;
    blob_id_from_value(scope, value)
}

fn blob_id_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<BlobId> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (n, _lossless) = big.u64_value();
        return (n >= 1).then_some(n);
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as BlobId)
}

pub(super) fn blob_bytes_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    blob_id_from_object(scope, object).and_then(blob_bytes)
}

pub(super) fn blob_uuid_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    blob_id_from_object(scope, object).and_then(blob_uuid)
}

pub(super) fn blob_bytes_for_uuid(
    partition_id: &RendererStoragePartitionIdentity,
    uuid: &str,
) -> Option<Arc<[u8]>> {
    blob_store().blob_shared_bytes_by_uuid_in_partition(uuid, partition_id)
}

pub(super) fn is_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    blob_id_from_object(scope, object).is_some()
}

pub(super) fn blob_mime_type_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    blob_id_from_object(scope, object).and_then(blob_mime_type)
}

pub(super) fn create_object_url_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    origin: &str,
) -> Option<String> {
    let blob_id = blob_id_from_object(scope, object)?;
    let owner_id = current_resource_owner_id(scope);
    let lifetime_id = native_bridge::current_runtime_observable_context_token(scope)
        .map(native_bridge::RuntimeObservableContextToken::as_u64);
    blob_store().create_object_url_with_lifetime(owner_id, lifetime_id, blob_id, origin)
}

pub(super) fn revoke_object_url(url: &str) {
    blob_store().revoke_object_url(url);
}

pub(super) fn object_url_body_and_type(url: &str) -> Option<(String, String)> {
    blob_store().object_url_body_and_type(url)
}

pub(super) fn object_url_bytes_and_type(url: &str) -> Option<(Vec<u8>, String)> {
    blob_store().object_url_bytes_and_type(url)
}

pub(super) fn collect_blob_bytes_and_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parts_value: v8::Local<'s, v8::Value>,
    options_value: v8::Local<'s, v8::Value>,
) -> Option<(Vec<u8>, String)> {
    collect_blob_bytes_and_type_with_options_context(
        scope,
        parts_value,
        options_value,
        "BlobPropertyBag",
        2,
    )
}

pub(super) fn collect_blob_bytes_and_type_with_options_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parts_value: v8::Local<'s, v8::Value>,
    options_value: v8::Local<'s, v8::Value>,
    options_context: &'static str,
    options_argument_index: usize,
) -> Option<(Vec<u8>, String)> {
    collect_blob_bytes_and_type_with_parts_mode(
        scope,
        parts_value,
        options_value,
        options_context,
        options_argument_index,
        BlobPartsMode::Optional,
    )
}

pub(crate) fn collect_required_blob_bytes_and_type_with_options_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parts_value: v8::Local<'s, v8::Value>,
    options_value: v8::Local<'s, v8::Value>,
    options_context: &'static str,
    options_argument_index: usize,
) -> Option<(Vec<u8>, String)> {
    collect_blob_bytes_and_type_with_parts_mode(
        scope,
        parts_value,
        options_value,
        options_context,
        options_argument_index,
        BlobPartsMode::Required,
    )
}

fn collect_blob_bytes_and_type_with_parts_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parts_value: v8::Local<'s, v8::Value>,
    options_value: v8::Local<'s, v8::Value>,
    options_context: &'static str,
    options_argument_index: usize,
    parts_mode: BlobPartsMode,
) -> Option<(Vec<u8>, String)> {
    let endings = match blob_property_bag_endings(
        scope,
        options_value,
        webidl::Context::argument(options_context, options_argument_index),
    ) {
        Ok(endings) => endings,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let bytes = match blob_part_sequence_bytes(scope, parts_value, parts_mode, endings) {
        Ok(bytes) => bytes,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let mime_type = match blob_property_bag_type(
        scope,
        options_value,
        webidl::Context::argument(options_context, options_argument_index),
    ) {
        Ok(mime_type) => mime_type,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };

    Some((bytes, mime_type))
}

fn blob_bytes(blob_id: BlobId) -> Option<Vec<u8>> {
    blob_store().blob_bytes(blob_id)
}

fn blob_uuid(blob_id: BlobId) -> Option<String> {
    blob_store().blob_uuid(blob_id)
}

fn blob_mime_type(blob_id: BlobId) -> Option<String> {
    blob_store().blob_mime_type(blob_id)
}

pub(crate) fn cleanup_owner_resources(owner_id: ResourceOwnerId) {
    blob_store().cleanup_owner_resources(owner_id);
}

pub(crate) fn cleanup_object_urls_for_context(
    context_token: native_bridge::RuntimeObservableContextToken,
) -> usize {
    blob_store().cleanup_object_url_lifetime(context_token.as_u64())
}

fn release_blob_wrapper_ref(blob_id: BlobId) {
    blob_store().release_blob_wrapper_ref(blob_id);
}

fn track_blob_ref_lifetime(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    finalizer: impl FnOnce() + 'static,
) {
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, object, finalizer);
}

fn blob_property_bag_endings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<BlobLineEndings, webidl::WebIdlError> {
    let Some(object) = webidl::dictionary_value(value, context)? else {
        return Ok(BlobLineEndings::Transparent);
    };
    let Some(value) = webidl::property_result(
        scope,
        object,
        "endings",
        webidl::Context::member("BlobPropertyBag", "endings"),
    )?
    else {
        return Ok(BlobLineEndings::Transparent);
    };
    if value.is_undefined() {
        return Ok(BlobLineEndings::Transparent);
    }
    webidl::convert::<webidl::EnumValue<BlobLineEndingsWebIdl>>(
        scope,
        value,
        webidl::Context::member("BlobPropertyBag", "endings"),
    )
    .map(|value| value.0.into())
}

fn blob_property_bag_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<String, webidl::WebIdlError> {
    let Some(object) = webidl::dictionary_value(value, context)? else {
        return Ok(String::new());
    };
    let Some(value) = webidl::property_result(
        scope,
        object,
        "type",
        webidl::Context::member("BlobPropertyBag", "type"),
    )?
    else {
        return Ok(String::new());
    };
    if value.is_undefined() {
        return Ok(String::new());
    }
    let value = webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("BlobPropertyBag", "type"),
    )?;
    Ok(normalize_blob_mime_type(&value.0))
}

fn bytes_from_blob_part<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    endings: BlobLineEndings,
) -> Result<Vec<u8>, webidl::WebIdlError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(bytes) = blob_bytes_from_object(scope, object)
    {
        return Ok(bytes);
    }
    if let Some(bytes) = buffer_source_bytes_from_value(scope, value) {
        return Ok(bytes);
    }
    let text = webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member("BlobPart", "value"),
    )?
    .0;
    Ok(
        normalize_blob_line_endings_with_native_ending(&text, endings, native_blob_line_ending())
            .into_bytes(),
    )
}

#[derive(Clone, Copy)]
enum BlobPartsMode {
    Optional,
    Required,
}

fn blob_part_sequence_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    mode: BlobPartsMode,
    endings: BlobLineEndings,
) -> Result<Vec<u8>, webidl::WebIdlError> {
    if matches!(mode, BlobPartsMode::Optional) && value.is_undefined() {
        return Ok(Vec::new());
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts must be an iterable object.",
        ));
    };
    if is_platform_indexed_blob_parts_object(scope, object)? {
        return blob_part_array_like_bytes(scope, object, endings);
    }
    let iterator_key = v8::Symbol::get_iterator(scope);
    let Some(iterator_value) = webidl::symbol_property_result(
        scope,
        object,
        iterator_key,
        webidl::Context::member("BlobPart", "@@iterator"),
    )?
    else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts must be an iterable object.",
        ));
    };
    if iterator_value.is_null_or_undefined() {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts must be an iterable object.",
        ));
    }
    let Ok(iterator_method) = v8::Local::<v8::Function>::try_from(iterator_value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts @@iterator must be callable.",
        ));
    };
    let Some(iterator_value) = call_function_result(
        scope,
        iterator_method,
        value,
        &[],
        webidl::Context::member("BlobPart", "@@iterator"),
    )?
    else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts @@iterator must return an iterator.",
        ));
    };
    let Ok(iterator) = v8::Local::<v8::Object>::try_from(iterator_value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts @@iterator must return an iterator.",
        ));
    };
    let Some(next_method) = webidl::property_result(
        scope,
        iterator,
        "next",
        webidl::Context::member("BlobPart", "next"),
    )?
    else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts iterator must have next().",
        ));
    };
    let Ok(next_method) = v8::Local::<v8::Function>::try_from(next_method) else {
        return Err(webidl::WebIdlError::custom_message(
            "Blob parts iterator next must be callable.",
        ));
    };

    let mut bytes = Vec::new();
    loop {
        let Some(step_value) = call_function_result(
            scope,
            next_method,
            iterator.into(),
            &[],
            webidl::Context::member("BlobPart", "next"),
        )?
        else {
            return Err(webidl::WebIdlError::custom_message(
                "Blob parts iterator next() must return an object.",
            ));
        };
        let Ok(step) = v8::Local::<v8::Object>::try_from(step_value) else {
            return Err(webidl::WebIdlError::custom_message(
                "Blob parts iterator next() must return an object.",
            ));
        };
        let done = webidl::property_result(
            scope,
            step,
            "done",
            webidl::Context::member("BlobPart", "done"),
        )?
        .is_some_and(|value| value.boolean_value(scope));
        if done {
            break;
        }
        let Some(part) = webidl::property_result(
            scope,
            step,
            "value",
            webidl::Context::member("BlobPart", "value"),
        )?
        else {
            return Err(webidl::WebIdlError::custom_message(
                "Blob parts iterator result is missing value.",
            ));
        };
        bytes.extend(bytes_from_blob_part(scope, part, endings)?);
    }
    Ok(bytes)
}

fn blob_part_array_like_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut object: v8::Local<'s, v8::Object>,
    endings: BlobLineEndings,
) -> Result<Vec<u8>, webidl::WebIdlError> {
    if blob_platform_indexed_object_kind(scope, object)
        == Some(BlobPlatformIndexedObjectKind::HtmlSelectElement)
    {
        if let Some(options) = webidl::property_result(
            scope,
            object,
            "options",
            webidl::Context::member("BlobPart", "options"),
        )?
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            object = options;
        } else if let Some(children) = webidl::property_result(
            scope,
            object,
            "children",
            webidl::Context::member("BlobPart", "children"),
        )?
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            object = children;
        }
    }
    let mut bytes = Vec::new();
    let length_value = webidl::property_result(
        scope,
        object,
        "length",
        webidl::Context::member("BlobPart", "length"),
    )?
    .ok_or_else(|| webidl::WebIdlError::custom_message("Blob parts array-like missing length."))?;
    let length = webidl::convert::<webidl::UnsignedLong>(
        scope,
        length_value,
        webidl::Context::member("BlobPart", "length"),
    )?
    .0;
    for index in 0..length {
        let Some(part) = get_index_result(scope, object, index)? else {
            continue;
        };
        bytes.extend(bytes_from_blob_part(scope, part, endings)?);
    }
    Ok(bytes)
}

fn is_platform_indexed_blob_parts_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<bool, webidl::WebIdlError> {
    Ok(blob_platform_indexed_object_kind(scope, object).is_some())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlobPlatformIndexedObjectKind {
    Collection,
    HtmlSelectElement,
    NamedNodeMap,
    FileList,
    DomStringList,
}

fn blob_platform_indexed_object_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobPlatformIndexedObjectKind> {
    if native_bridge::blob_parts_platform_collection_kind(scope, object).is_some() {
        return Some(BlobPlatformIndexedObjectKind::Collection);
    }
    if let Ok((runtime_ptr, handle)) =
        native_bridge::node_runtime_and_handle_from_object(scope, object)
        && unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "select")
    {
        return Some(BlobPlatformIndexedObjectKind::HtmlSelectElement);
    }
    match object
        .get_constructor_name()
        .to_rust_string_lossy(scope)
        .as_str()
    {
        "NamedNodeMap" => Some(BlobPlatformIndexedObjectKind::NamedNodeMap),
        "FileList" => Some(BlobPlatformIndexedObjectKind::FileList),
        "DOMStringList" => Some(BlobPlatformIndexedObjectKind::DomStringList),
        "HTMLCollection"
        | "HTMLFormControlsCollection"
        | "HTMLOptionsCollection"
        | "NodeList"
        | "RadioNodeList" => Some(BlobPlatformIndexedObjectKind::Collection),
        "HTMLSelectElement" => Some(BlobPlatformIndexedObjectKind::HtmlSelectElement),
        _ => None,
    }
}

fn get_index_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match object.get_index(&scope, index) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(webidl::WebIdlError::pending_exception(
                webidl::Context::member("BlobPart", "sequence item"),
            ))
        }
        None => Ok(None),
    }
}

fn call_function_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    context: webidl::Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match function.call(&scope, receiver, args) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(webidl::WebIdlError::pending_exception(context))
        }
        None => Ok(None),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, webidl::WebIdlEnum)]
#[webidl(name = "EndingType")]
enum BlobLineEndingsWebIdl {
    Transparent,
    Native,
}

impl From<BlobLineEndingsWebIdl> for BlobLineEndings {
    fn from(value: BlobLineEndingsWebIdl) -> Self {
        match value {
            BlobLineEndingsWebIdl::Transparent => Self::Transparent,
            BlobLineEndingsWebIdl::Native => Self::Native,
        }
    }
}

fn clamped_long_long_arg(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<i64> {
    args.get(index)
        .number_value(scope)
        .map(clamp_blob_long_long)
}

fn blob_slice_content_type(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<String> {
    if args.length() <= 2 || args.get(2).is_undefined() {
        return Some(String::new());
    }
    let value = args.get(2).to_string(scope)?;
    Some(normalize_blob_mime_type(
        value.to_rust_string_lossy(scope).as_str(),
    ))
}

pub(super) fn buffer_source_bytes_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<Vec<u8>> {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    None
}

pub(super) fn buffer_source_has_shared_or_resizable_backing_store(
    value: v8::Local<'_, v8::Value>,
) -> bool {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let backing_store = buffer.get_backing_store();
        return backing_store.is_shared() || backing_store.is_resizable_by_user_javascript();
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value)
        && let Some(backing_store) = view.get_backing_store()
    {
        return backing_store.is_shared() || backing_store.is_resizable_by_user_javascript();
    }
    false
}

pub(super) fn array_buffer_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::ArrayBuffer>> {
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    Some(v8::ArrayBuffer::with_backing_store(scope, &backing_store))
}

fn new_uint8_array_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let len = bytes.len();
    let buffer = array_buffer_from_bytes(scope, bytes)?;
    v8::Uint8Array::new(scope, buffer, 0, len)
}

fn set_resolved_promise(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

fn set_rejected_type_error_promise(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    message: &str,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    let reason = v8::Exception::type_error(scope, message);
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}

//! V8 Blob bindings. The embedder supplies backing storage and schedules reads.

use crate::interfaces as web_api_interfaces;
use moli_file_api::{BlobData, BlobId, blob_slice_relative_index, normalize_blob_mime_type};
use moli_v8_util::{get_private_value, global_constructor_prototype, throw_type_error, v8_string};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;

const BLOB_ID_SLOT: &str = "__lmBlobId";

pub(crate) mod parts;
use parts::{BlobParts, BlobPropertyBag};

pub enum BlobReadMode {
    Text,
    ArrayBuffer,
    Bytes,
}

pub struct BlobRuntime {
    pub allocate: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, BlobData, String) -> BlobId,
    pub data: fn(BlobId) -> Option<BlobData>,
    pub size: fn(BlobId) -> Option<usize>,
    pub mime_type: fn(BlobId) -> Option<String>,
    pub release_wrapper: fn(BlobId),
    pub track_wrapper: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>, BlobId),
    pub read: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        BlobReadMode,
    ) -> Option<v8::Local<'s, v8::Promise>>,
    pub stream: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>>,
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> Option<&'static BlobRuntime> {
    Some(&crate::embedding::realm_runtime(scope)?.blob)
}

fn read_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mode: BlobReadMode,
) -> Option<v8::Local<'s, v8::Promise>> {
    (runtime(scope)?.read)(scope, object, mode)
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Blob, enumerable, receiver)]
struct BlobTemplateMethodsDeclaration {
    #[webapi(method, length = 0, returns_promise, callback = blob_text_callback)]
    text: (),

    #[webapi(method, length = 0, returns_promise, callback = blob_array_buffer_callback)]
    array_buffer: (),

    #[webapi(method, length = 0, returns_promise, callback = blob_bytes_callback)]
    bytes: (),

    #[webapi(method, length = 0, callback = blob_stream_callback)]
    stream: (),

    #[webapi(method, length = 0, callback = blob_slice_callback)]
    slice: (),
}

pub fn install_blob_methods<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    BlobTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Blob")]
struct BlobConstructorArgs<'s> {
    #[webidl(with = parts::optional_blob_parts_argument)]
    parts: BlobParts<'s>,
    #[webidl(dictionary)]
    options: BlobPropertyBag,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Blob.slice")]
struct BlobSliceArgs {
    #[webidl(converter = "clamped_long_long")]
    start: Option<i64>,
    #[webidl(converter = "clamped_long_long")]
    end: Option<i64>,
    #[webidl(default = String::new())]
    content_type: String,
}

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
#[webapi(prototype = "Object", interface = web_api_interfaces::Blob)]
struct BlobInstanceDeclaration<'scope> {
    #[webapi(slot = BLOB_ID_SLOT)]
    blob_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Blob, enumerable, receiver)]
struct BlobPrototypeDeclaration {
    #[webapi(accessor_property, getter = blob_size_attribute_getter_callback)]
    size: (),
    #[webapi(accessor_property, getter = blob_type_attribute_getter_callback)]
    r#type: (),
}

pub fn blob_constructor_callback<'s>(
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

    let Some(parsed) = webidl::parse_args::<BlobConstructorArgs>(scope, &args) else {
        return;
    };
    let bytes = match parsed.parts.into_data(scope, parsed.options.line_endings()) {
        Ok(bytes) => bytes,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mime_type = normalize_blob_mime_type(&parsed.options.r#type);

    init_blob_object_from_data(scope, args.this(), bytes, mime_type);
    rv.set(args.this().into());
}

pub fn install_blob_template_bindings<'s>(
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

fn blob_size_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let size = blob_size_from_object(scope, args.this())
        .map(|size| size as f64)
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, size).into());
}

fn blob_type_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let mime_type = blob_mime_type_from_object(scope, args.this()).unwrap_or_default();
    if let Some(value) = v8_string(scope, &mime_type) {
        rv.set(value.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

fn blob_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(promise) = read_blob(scope, args.this(), BlobReadMode::Text) {
        rv.set(promise.into());
    }
}

fn blob_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(promise) = read_blob(scope, args.this(), BlobReadMode::ArrayBuffer) {
        rv.set(promise.into());
    }
}

fn blob_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(promise) = read_blob(scope, args.this(), BlobReadMode::Bytes) {
        rv.set(promise.into());
    }
}

fn blob_stream_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) = runtime(scope).and_then(|runtime| (runtime.stream)(scope, args.this())) {
        rv.set(stream.into());
    }
}

fn blob_slice_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<BlobSliceArgs>(scope, &args) else {
        return;
    };
    let Some(bytes) = blob_data_from_object(scope, args.this()) else {
        return;
    };
    let size = bytes.len();
    let start = parsed.start.unwrap_or(0);
    let end = parsed.end.unwrap_or(size as i64);
    let mime_type = normalize_blob_mime_type(&parsed.content_type);

    let relative_start = blob_slice_relative_index(start, size);
    let relative_end = blob_slice_relative_index(end, size);
    let span = relative_end.saturating_sub(relative_start);
    let sliced = bytes.slice(relative_start..relative_start + span);

    // Convert arguments in the method's realm, then allocate in the receiver's realm.
    let Some(receiver_context) = args.this().get_creation_context(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, receiver_context);
    if let Some(blob) = build_blob_object_from_data(scope, sliced, mime_type) {
        rv.set(blob.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub fn init_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
    mime_type: String,
) -> BlobId {
    init_blob_object_from_data(scope, object, bytes.into(), mime_type)
}

pub fn init_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: BlobData,
    mime_type: String,
) -> BlobId {
    try_init_blob_object_from_data(scope, object, bytes, mime_type)
        .expect("Blob instance declaration should initialize")
}

pub fn try_init_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: BlobData,
    mime_type: String,
) -> Option<BlobId> {
    let runtime = runtime(scope)?;
    let blob_id = (runtime.allocate)(scope, bytes, mime_type);
    if BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .initialize(scope, object)
        .is_err()
    {
        (runtime.release_wrapper)(blob_id);
        return None;
    }
    (runtime.track_wrapper)(scope, object, blob_id);
    Some(blob_id)
}

pub fn build_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    build_blob_object_from_data(scope, bytes.into(), mime_type)
}

pub fn build_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: BlobData,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    build_blob_object_unpublished(scope, bytes, mime_type).map(|(object, _)| object)
}

pub fn build_blob_object_unpublished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: BlobData,
    mime_type: String,
) -> Option<(v8::Local<'s, v8::Object>, BlobId)> {
    let prototype = global_constructor_prototype(scope, "Blob")?;
    let runtime = runtime(scope)?;
    let blob_id = (runtime.allocate)(scope, bytes, mime_type);
    let object = BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .bind(scope)
        .ok()
        .or_else(|| {
            (runtime.release_wrapper)(blob_id);
            None
        })?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        (runtime.release_wrapper)(blob_id);
        return None;
    }
    (runtime.track_wrapper)(scope, object, blob_id);
    Some((object, blob_id))
}

pub fn blob_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobId> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let value = get_private_value(scope, object, BLOB_ID_SLOT)?;
    blob_id_from_value(scope, value)
}

pub fn world_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = blob_id_from_object(scope, source)?;
    // The wrapper cache binds this view to source, retaining its native Blob
    // reference. No payload copy or independent Blob identity is created.
    let object = BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, id))
        .bind(scope)
        .ok()?;
    let prototype = global_constructor_prototype(scope, "Blob")?;
    object
        .set_prototype(scope, prototype.into())?
        .then_some(())?;
    Some(object)
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

pub fn blob_data_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobData> {
    (runtime(scope)?.data)(blob_id_from_object(scope, object)?)
}

pub fn is_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::Blob::is_instance(scope, object)
}

pub fn blob_size_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    (runtime(scope)?.size)(blob_id_from_object(scope, object)?)
}

pub fn blob_mime_type_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    (runtime(scope)?.mime_type)(blob_id_from_object(scope, object)?)
}

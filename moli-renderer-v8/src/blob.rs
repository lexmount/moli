use crate::context_bootstrap::media_source::{MediaSourceObject, media_source_object};
use crate::{native_bridge, web_api_interfaces};
use moli_file_api::{
    BlobData, BlobId, BlobStore, ObjectUrlEntry, ObjectUrlTarget, blob_slice_relative_index,
    normalize_blob_mime_type,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::sync::{Arc, OnceLock};

use super::{
    resource_owner::{ResourceOwnerId, current_resource_owner_id},
    runtime::RendererStoragePartitionIdentity,
    util::{get_private_value, throw_type_error, v8_string},
    webidl,
};

const BLOB_ID_SLOT: &str = "__lmBlobId";

mod parts;
pub(crate) mod read;
mod stream;
pub(crate) use parts::{BlobParts, BlobPropertyBag, required_blob_parts_argument};
pub(crate) use read::BlobReadTask;
pub(crate) use stream::create as stream_blob;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectUrlAccessKey {
    partition: RendererStoragePartitionIdentity,
    storage_key: moli_storage_key::MoliStorageKey,
}

impl ObjectUrlAccessKey {
    pub(crate) fn new(
        partition: RendererStoragePartitionIdentity,
        storage_key: moli_storage_key::MoliStorageKey,
    ) -> Self {
        Self {
            partition,
            storage_key,
        }
    }

    pub(crate) fn for_worker(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        Some(Self::new(
            crate::worker::worker_storage_partition_identity(scope)?,
            crate::worker::worker_storage_key(scope)?,
        ))
    }

    pub(crate) fn same_partition(&self, other: &Self) -> bool {
        self.partition == other.partition
    }
}

type RendererBlobStore = BlobStore<
    ResourceOwnerId,
    RendererStoragePartitionIdentity,
    ObjectUrlAccessKey,
    ObjectUrlPolicySource,
    Arc<MediaSourceObject>,
>;

#[derive(Clone, Debug)]
pub(crate) struct ObjectUrlPolicySource {
    pub(crate) content_security_policy: crate::content_security_policy::ContentSecurityPolicySource,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) document_meta_referrer_policy:
        Option<moli_dom::native::DocumentMetaReferrerPolicySource>,
}

#[derive(Default)]
pub(crate) struct ObjectUrlPolicySnapshot {
    pub(crate) content_security_policy:
        crate::content_security_policy::InheritedContentSecurityPolicy,
    pub(crate) referrer_policy: Option<String>,
}

static BLOB_STORE: OnceLock<RendererBlobStore> = OnceLock::new();

fn blob_store() -> &'static RendererBlobStore {
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

pub(super) fn blob_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    read::start(scope, args.this(), read::BlobReadFormat::Text, &mut rv);
}

pub(super) fn blob_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    read::start(
        scope,
        args.this(),
        read::BlobReadFormat::ArrayBuffer,
        &mut rv,
    );
}

pub(super) fn blob_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    read::start(scope, args.this(), read::BlobReadFormat::Bytes, &mut rv);
}

pub(super) fn blob_stream_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(stream) = stream::create(scope, args.this()) {
        rv.set(stream.into());
    }
}

pub(super) fn blob_slice_callback<'s>(
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

pub(super) fn init_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
    mime_type: String,
) -> BlobId {
    try_init_blob_object_from_shared(scope, object, bytes.into(), mime_type)
        .expect("Blob instance declaration should initialize")
}

pub(super) fn try_init_blob_object_from_shared<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: std::sync::Arc<[u8]>,
    mime_type: String,
) -> Option<BlobId> {
    try_init_blob_object_from_data(scope, object, bytes.into(), mime_type)
}

pub(super) fn init_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: BlobData,
    mime_type: String,
) -> BlobId {
    try_init_blob_object_from_data(scope, object, bytes, mime_type)
        .expect("Blob instance declaration should initialize")
}

pub(super) fn try_init_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    bytes: BlobData,
    mime_type: String,
) -> Option<BlobId> {
    let owner_id = current_resource_owner_id(scope);
    let partition_id = current_blob_storage_partition_identity(scope);
    let blob_id = blob_store().create_blob_from_data(owner_id, partition_id, bytes, mime_type);
    if BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .initialize(scope, object)
        .is_err()
    {
        release_blob_wrapper_ref(blob_id);
        return None;
    }
    track_blob_ref_lifetime(scope, object, move || release_blob_wrapper_ref(blob_id));
    Some(blob_id)
}

pub(super) fn build_blob_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    build_blob_object_from_shared(scope, bytes.into(), mime_type)
}

pub(super) fn build_blob_object_from_shared<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: std::sync::Arc<[u8]>,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    build_blob_object_from_data(scope, bytes.into(), mime_type)
}

pub(super) fn build_blob_object_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: BlobData,
    mime_type: String,
) -> Option<v8::Local<'s, v8::Object>> {
    build_blob_object_unpublished(scope, bytes, mime_type).map(|(object, _)| object)
}

pub(super) fn build_blob_object_unpublished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: BlobData,
    mime_type: String,
) -> Option<(v8::Local<'s, v8::Object>, BlobId)> {
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "Blob").ok()?;
    let owner_id = current_resource_owner_id(scope);
    let partition_id = current_blob_storage_partition_identity(scope);
    let blob_id = blob_store().create_blob_from_data(owner_id, partition_id, bytes, mime_type);
    let object = BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, blob_id))
        .bind(scope)
        .ok()
        .or_else(|| {
            release_blob_wrapper_ref(blob_id);
            None
        })?;
    let _ = object.set_prototype(scope, prototype.into());
    track_blob_ref_lifetime(scope, object, move || release_blob_wrapper_ref(blob_id));
    Some((object, blob_id))
}

#[cfg(test)]
pub(crate) fn live_blob_count_for_test() -> usize {
    blob_store().live_blob_count()
}

pub(crate) fn blob_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobId> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let value = get_private_value(scope, object, BLOB_ID_SLOT)?;
    blob_id_from_value(scope, value)
}

pub(crate) fn world_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = blob_id_from_object(scope, source)?;
    // The wrapper cache binds this view to source, retaining its native Blob
    // reference. No payload copy or independent Blob identity is created.
    let object = BlobInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, id))
        .bind(scope)
        .ok()?;
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "Blob").ok()?;
    object.set_prototype(scope, prototype.into())?;
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

pub(super) fn blob_data_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BlobData> {
    blob_store().blob_data(blob_id_from_object(scope, object)?)
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
    web_api_interfaces::Blob::is_instance(scope, object)
}

pub(super) fn blob_size_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    blob_id_from_object(scope, object).and_then(|id| blob_store().blob_size(id))
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
    storage_key: moli_storage_key::MoliStorageKey,
) -> Option<String> {
    let target = if is_blob_object(scope, object) {
        ObjectUrlTarget::Blob(blob_id_from_object(scope, object)?)
    } else {
        ObjectUrlTarget::MediaSource(media_source_object(scope, object)?)
    };
    let partition = current_blob_storage_partition_identity(scope)?;
    let owner_id = current_resource_owner_id(scope);
    let origin = storage_key.origin().to_owned();
    let lifetime_id = native_bridge::current_runtime_observable_context_token(scope)
        .map(native_bridge::RuntimeObservableContextToken::as_u64);
    let policy_source =
        if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
            let global = scope.get_current_context().global(scope);
            // SAFETY: the Window callback keeps its host alive for this call.
            unsafe { &*host_ptr }.object_url_policy_source_for_global(scope, global)
        } else {
            crate::worker::worker_content_security_policy_snapshot(scope).map(|policy| {
                ObjectUrlPolicySource {
                    content_security_policy: Arc::new(parking_lot::RwLock::new(policy)),
                    referrer_policy: crate::worker::worker_referrer_policy(scope),
                    document_meta_referrer_policy: None,
                }
            })
        };
    blob_store().create_object_url_with_target(
        owner_id,
        lifetime_id,
        target,
        &origin,
        Some(ObjectUrlAccessKey {
            partition,
            storage_key,
        }),
        policy_source,
    )
}

pub(crate) type RendererObjectUrlEntry = ObjectUrlEntry<ObjectUrlAccessKey, Arc<MediaSourceObject>>;

pub(crate) fn object_url_entry(url: &str) -> Option<RendererObjectUrlEntry> {
    blob_store().object_url_entry(url)
}

pub(crate) fn object_url_resource_entry(
    url: &str,
) -> Option<moli_file_api::ObjectUrlResourceEntry<ObjectUrlAccessKey>> {
    blob_store().object_url_resource_entry(url)
}

pub(crate) fn object_url_body_and_type_with_access_key(
    url: &str,
    access_key: &ObjectUrlAccessKey,
) -> Option<(String, String)> {
    let moli_file_api::ObjectUrlData::Blob { bytes, mime_type } = blob_store()
        .object_url_entry_with_access_key(url, access_key)?
        .data
    else {
        return None;
    };
    Some((String::from_utf8_lossy(&bytes).into_owned(), mime_type))
}

pub(crate) fn object_url_policy(url: &str) -> Option<ObjectUrlPolicySnapshot> {
    let source = blob_store().object_url_metadata(url)?;
    Some(ObjectUrlPolicySnapshot {
        content_security_policy: source.content_security_policy.read().clone(),
        referrer_policy: source
            .document_meta_referrer_policy
            .as_ref()
            .and_then(moli_dom::native::DocumentMetaReferrerPolicySource::get)
            .map(str::to_owned)
            .or(source.referrer_policy),
    })
}

pub(super) fn revoke_object_url(
    scope: &mut v8::PinScope<'_, '_>,
    url: &str,
    storage_key: moli_storage_key::MoliStorageKey,
) {
    let Some(partition) = current_blob_storage_partition_identity(scope) else {
        return;
    };
    blob_store().revoke_object_url_with_access_key(
        url,
        &ObjectUrlAccessKey {
            partition,
            storage_key,
        },
    );
}

pub(super) fn object_url_body_and_type(url: &str) -> Option<(String, String)> {
    blob_store().object_url_body_and_type(url)
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
    blob_store().retire_owner_resources(owner_id);
}

pub(crate) fn cleanup_object_urls_for_context(
    owner_id: ResourceOwnerId,
    context_token: native_bridge::RuntimeObservableContextToken,
) -> usize {
    blob_store().cleanup_object_url_lifetime(owner_id, context_token.as_u64())
}

pub(super) fn release_blob_wrapper_ref(blob_id: BlobId) {
    blob_store().release_blob_wrapper_ref(blob_id);
}

fn track_blob_ref_lifetime(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    finalizer: impl FnOnce() + 'static,
) {
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, object, finalizer);
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

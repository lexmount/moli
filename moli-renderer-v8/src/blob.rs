use crate::context_bootstrap::media_source::{MediaSourceObject, media_source_object};
use crate::native_bridge;
use moli_file_api::{BlobData, BlobId, BlobStore, ObjectUrlEntry, ObjectUrlTarget};
pub(super) use moli_v8_util::buffer_source_bytes_from_value;
use moli_webapi::blob::{BlobReadMode, BlobRuntime};
pub(super) use moli_webapi::blob::{
    blob_constructor_callback, blob_data_from_object, blob_mime_type_from_object,
    blob_size_from_object, build_blob_object, build_blob_object_from_data,
    build_blob_object_unpublished, install_blob_template_bindings, is_blob_object,
};
pub(crate) use moli_webapi::blob::{blob_id_from_object, world_wrapper};
use std::sync::{Arc, OnceLock};

use super::{
    resource_owner::{ResourceOwnerId, current_resource_owner_id},
    runtime::RendererStoragePartitionIdentity,
};

pub(crate) mod read;
mod reading;
mod stream;
pub(crate) use reading::{BlobReadTask, stream_blob};

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

pub(crate) const RUNTIME: BlobRuntime = BlobRuntime {
    allocate: |scope, bytes, mime_type| {
        let owner_id = current_resource_owner_id(scope);
        let partition_id = current_blob_storage_partition_identity(scope);
        blob_store().create_blob_from_data(owner_id, partition_id, bytes, mime_type)
    },
    data: |id| blob_store().blob_data(id),
    size: |id| blob_store().blob_size(id),
    mime_type: |id| blob_store().blob_mime_type(id),
    release_wrapper: release_blob_wrapper_ref,
    track_wrapper: |scope, blob, id| {
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, blob, move || {
            release_blob_wrapper_ref(id);
        });
    },
    read: reading::read_blob,
    stream: reading::stream_blob,
};

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

#[cfg(test)]
pub(crate) fn live_blob_count_for_test() -> usize {
    blob_store().live_blob_count()
}

#[cfg(feature = "devtools")]
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

pub(crate) fn object_url_download_data(
    host: &mut crate::native_bridge::PageScriptHost,
    handle: crate::document_runtime::DomHandle,
    url: &str,
) -> Option<(BlobData, String)> {
    use crate::native_bridge::OwnerDispatchScope;

    // The link's Document owns the request, even when click() is called from
    // another realm. Never authorize through the ambient JS context.
    let context = match host.owner_dispatch_scope_for_node(handle)? {
        OwnerDispatchScope::Top => host.top_document_storage_context(),
        OwnerDispatchScope::Child(handle) => {
            host.storage_context_for_child_browsing_context(handle)?
        }
        OwnerDispatchScope::LightweightPopup(id) => {
            host.storage_context_for_lightweight_popup(id)?
        }
    };
    let key = ObjectUrlAccessKey {
        partition: host.browser_context_runtime().storage_partition_identity(),
        storage_key: context.storage_key().clone(),
    };
    blob_store().object_url_data_with_access_key(url, &key)
}

pub(super) fn object_url_body_and_type(url: &str) -> Option<(String, String)> {
    blob_store().object_url_body_and_type(url)
}

#[cfg(feature = "devtools")]
fn blob_uuid(blob_id: BlobId) -> Option<String> {
    blob_store().blob_uuid(blob_id)
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

pub(super) use moli_v8_util::{
    array_buffer_from_bytes, buffer_source_has_shared_or_resizable_backing_store,
};

fn new_uint8_array_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let len = bytes.len();
    let buffer = array_buffer_from_bytes(scope, bytes)?;
    v8::Uint8Array::new(scope, buffer, 0, len)
}

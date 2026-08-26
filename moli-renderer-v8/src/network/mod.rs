pub use crate::{
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord,
    WebStorageMutationSubscription, WebStorageString, deep_clone_shared_web_storage_store,
    new_shared_json_web_storage_store, new_shared_web_storage_store,
    web_storage_partitioned_area_key,
};
use moli_fetch::RequestResourceType;

use crate::types::SubresourceResourceType;

mod backend;
pub mod context;
mod devtools_resource_load;
pub(crate) mod loads;
pub mod navigation;
mod policy;
mod request_client;
mod task_runner;

pub use backend::{
    BrowserParkableImageDiagnostics, BrowserResourceDiskPoolDiagnostics, BrowserResourceRuntime,
    BrowserResourceRuntimeDiagnostics, BrowserResourceRuntimeOwner,
    BrowserResourceRuntimeOwnerRegistrar, BrowserResourceRuntimeOwnerRegistration,
    SharedMemoryResourceCacheDiagnostics,
};
pub(crate) use backend::{BrowserResourceRuntimeBinding, BrowserResourceRuntimeOwnerRoot};
pub use devtools_resource_load::{
    RendererNetworkResourceLoadOutcome, RendererNetworkResourceLoadPreparation,
    RendererNetworkResourceLoadResponse, RendererPreparedNetworkResourceLoad,
};
pub use policy::{PageNetworkPolicy, PageNetworkPolicySnapshot};
pub use request_client::{ResourceRequestClient, ResourceRequestClientOwner};
pub use task_runner::RendererResourceTaskRunner;

pub(crate) fn request_resource_type_for_subresource(
    resource_type: SubresourceResourceType,
) -> Option<RequestResourceType> {
    match resource_type {
        SubresourceResourceType::Script => Some(RequestResourceType::Script),
        SubresourceResourceType::Stylesheet => Some(RequestResourceType::CssStyleSheet),
        SubresourceResourceType::Image => Some(RequestResourceType::Image),
        SubresourceResourceType::Font => Some(RequestResourceType::Font),
        SubresourceResourceType::Audio
        | SubresourceResourceType::Video
        | SubresourceResourceType::Media => Some(RequestResourceType::Media),
        SubresourceResourceType::TextTrack => Some(RequestResourceType::TextTrack),
        SubresourceResourceType::Ping => Some(RequestResourceType::Ping),
        SubresourceResourceType::CspReport => Some(RequestResourceType::CspReport),
        SubresourceResourceType::Dictionary => Some(RequestResourceType::Dictionary),
        SubresourceResourceType::Manifest => Some(RequestResourceType::Manifest),
        SubresourceResourceType::Fetch
        | SubresourceResourceType::EventSource
        | SubresourceResourceType::Xhr
        | SubresourceResourceType::WebSocket => None,
    }
}

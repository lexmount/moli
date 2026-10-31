pub use moli_renderer_v8::network::{
    BrowserResourceRuntime, BrowserResourceRuntimeOwner, ResourceRequestClient,
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord,
    WebStorageMutationSubscription, WebStorageString,
    context::{DocumentResourceLoader, DocumentResourceLoaderDiagnostics},
    deep_clone_shared_web_storage_store,
    navigation::{
        DocumentFetchContextSeed, NavigationResourceLoader, NavigationResourceLoaderDiagnostics,
        NavigationResourceLoaderState,
    },
    new_shared_web_storage_store, web_storage_partitioned_area_key,
};

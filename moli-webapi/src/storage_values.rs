//! Value projection for StorageManager's asynchronous estimate.

use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(fragment, prototype = "StorageEstimate")]
struct StorageEstimateObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    quota: f64,

    #[webapi(data_property, enumerable)]
    usage: f64,

    #[webapi(data_property, enumerable)]
    usage_details: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct StorageUsageDetailsObjectDeclaration {
    #[webapi(data_property = "indexedDB", enumerable)]
    indexed_db: Option<f64>,
    #[webapi(data_property = "caches", enumerable)]
    caches: Option<f64>,
    #[webapi(data_property = "fileSystem", enumerable)]
    file_system: Option<f64>,
}

pub fn new_storage_estimate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    usage: u64,
    indexed_db_usage: u64,
    cache_storage_usage: u64,
    opfs_usage: u64,
    quota: u64,
) -> v8::Local<'s, v8::Object> {
    let usage_details = build_storage_usage_details_object(
        scope,
        indexed_db_usage,
        cache_storage_usage,
        opfs_usage,
    );
    StorageEstimateObjectDeclaration::new(quota as f64, usage as f64, usage_details)
        .bind(scope)
        .expect("StorageEstimate declaration should bind")
}

fn build_storage_usage_details_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    indexed_db_usage: u64,
    cache_storage_usage: u64,
    opfs_usage: u64,
) -> v8::Local<'s, v8::Object> {
    StorageUsageDetailsObjectDeclaration::new(
        (indexed_db_usage > 0).then_some(indexed_db_usage as f64),
        (cache_storage_usage > 0).then_some(cache_storage_usage as f64),
        (opfs_usage > 0).then_some(opfs_usage as f64),
    )
    .bind(scope)
    .expect("StorageUsageDetails declaration should bind")
}

use moli_webidl as webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "StorageAccessTypes")]
pub struct StorageAccessTypes {
    #[webidl(default = false)]
    all: bool,
    #[webidl(default = false)]
    cookies: bool,
    #[webidl(name = "sessionStorage", default = false)]
    session_storage: bool,
    #[webidl(name = "localStorage", default = false)]
    local_storage: bool,
    #[webidl(name = "indexedDB", default = false)]
    indexed_db: bool,
    #[webidl(default = false)]
    locks: bool,
    #[webidl(default = false)]
    caches: bool,
    #[webidl(name = "getDirectory", default = false)]
    get_directory: bool,
    #[webidl(default = false)]
    estimate: bool,
    #[webidl(name = "createObjectURL", default = false)]
    create_object_url: bool,
    #[webidl(name = "revokeObjectURL", default = false)]
    revoke_object_url: bool,
    #[webidl(name = "BroadcastChannel", default = false)]
    broadcast_channel: bool,
    #[webidl(name = "SharedWorker", default = false)]
    shared_worker: bool,
}

impl StorageAccessTypes {
    pub fn requests_any_access(&self) -> bool {
        self.all
            || self.cookies
            || self.session_storage
            || self.local_storage
            || self.indexed_db
            || self.locks
            || self.caches
            || self.get_directory
            || self.estimate
            || self.create_object_url
            || self.revoke_object_url
            || self.broadcast_channel
            || self.shared_worker
    }

    pub fn requests_get_directory(&self) -> bool {
        self.all || self.get_directory
    }
}

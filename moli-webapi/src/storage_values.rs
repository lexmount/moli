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

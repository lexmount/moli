use super::*;
use moli_browser_profile::BrowserIdentityProfile;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

use crate::context_bootstrap::navigator_runtime::{
    STORAGE_BUCKET_MANAGER_BRAND_SLOT, STORAGE_BUCKET_MANAGER_CHILD_HANDLE_SLOT,
    STORAGE_BUCKET_MANAGER_POPUP_ID_SLOT, STORAGE_MANAGER_BRAND_SLOT,
    STORAGE_MANAGER_CHILD_HANDLE_SLOT, STORAGE_MANAGER_POPUP_ID_SLOT,
    current_protocol_user_gesture_activation, navigator_identity_profile,
    navigator_receiver_branded, set_navigator_identity_profile,
};
use crate::context_bootstrap::storage_buckets::{
    IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME, StorageBucketCacheId, StorageBucketCacheMatch,
    StorageBucketCachePutOutcome, StorageBucketCacheQuery, StorageBucketCachedRequest,
    StorageBucketCachedResponse, StorageBucketDurability, StorageBucketIdentity,
    complete_storage_bucket_deletion_for_context, current_storage_bucket_storage_key,
    current_storage_bucket_store, storage_bucket_origin_allows_storage,
    storage_bucket_quota_owner_for_locator, with_storage_bucket_store_entry,
};
use crate::document_runtime::DomHandle;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;

mod legacy_storage_quota;

pub(crate) use legacy_storage_quota::{
    LegacyStorageQuotaCallbackOutcome, LegacyStorageQuotaCallbackTask,
    LegacyStorageQuotaCallbackTaskEffect,
};
pub(in crate::context_bootstrap) use legacy_storage_quota::{
    build_legacy_storage_info_object, build_legacy_storage_quota_object,
};

const PERMISSION_STATUS_NAME_SLOT: &str = "__moliPermissionStatusName";
const PERMISSION_STATUS_STATE_SLOT: &str = "__moliPermissionStatusState";
const PERMISSION_STATUS_BRAND_SLOT: &str = "__moliPermissionStatusBrand";
const VIBRATION_PATTERN_LENGTH_MAX: usize = 99;
const VIBRATION_DURATION_MS_MAX: u32 = 10_000;
const STORAGE_BUCKET_BRAND_SLOT: &str = "__moliStorageBucketBrand";
const STORAGE_BUCKET_CACHES_OBJECT_SLOT: &str = "__moliStorageBucketCachesObject";
const STORAGE_BUCKET_ID_SLOT: &str = "__moliStorageBucketId";
const STORAGE_BUCKET_NAME_SLOT: &str = "__moliStorageBucketName";
const STORAGE_BUCKET_ORIGIN_SLOT: &str = "__moliStorageBucketOrigin";
const STORAGE_BUCKET_STORAGE_KEY_SLOT: &str = "__moliStorageBucketStorageKey";
const STORAGE_BUCKET_CACHE_BRAND_SLOT: &str = "__moliStorageBucketCacheBrand";
const STORAGE_BUCKET_CACHE_NAME_SLOT: &str = "__moliStorageBucketCacheName";
const STORAGE_BUCKET_CACHE_ID_SLOT: &str = "__moliStorageBucketCacheId";
const STORAGE_BUCKET_CACHE_STORAGE_BRAND_SLOT: &str = "__moliStorageBucketCacheStorageBrand";
const GLOBAL_CACHE_STORAGE_SLOT: &str = "__moliGlobalCacheStorage";
const STORAGE_BUCKET_CACHE_PUT_RESOLVER_SLOT: &str = "__moliStorageBucketCachePutResolver";
const STORAGE_BUCKET_CACHE_PUT_BUCKET_ORIGIN_SLOT: &str = "__moliStorageBucketCachePutBucketOrigin";
const STORAGE_BUCKET_CACHE_PUT_BUCKET_NAME_SLOT: &str = "__moliStorageBucketCachePutBucketName";
const STORAGE_BUCKET_CACHE_PUT_BUCKET_ID_SLOT: &str = "__moliStorageBucketCachePutBucketId";
const STORAGE_BUCKET_CACHE_PUT_BUCKET_STORAGE_KEY_SLOT: &str =
    "__moliStorageBucketCachePutBucketStorageKey";
const STORAGE_BUCKET_CACHE_PUT_CACHE_NAME_SLOT: &str = "__moliStorageBucketCachePutCacheName";
const STORAGE_BUCKET_CACHE_PUT_CACHE_ID_SLOT: &str = "__moliStorageBucketCachePutCacheId";
const STORAGE_BUCKET_CACHE_PUT_REQUEST_KEY_SLOT: &str = "__moliStorageBucketCachePutRequestKey";
const STORAGE_BUCKET_CACHE_PUT_REQUEST_METHOD_SLOT: &str =
    "__moliStorageBucketCachePutRequestMethod";
const STORAGE_BUCKET_CACHE_PUT_REQUEST_HEADERS_SLOT: &str =
    "__moliStorageBucketCachePutRequestHeaders";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_TYPE_SLOT: &str = "__moliStorageBucketCachePutResponseType";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_URL_SLOT: &str = "__moliStorageBucketCachePutResponseUrl";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_REDIRECTED_SLOT: &str =
    "__moliStorageBucketCachePutResponseRedirected";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_SLOT: &str =
    "__moliStorageBucketCachePutResponseStatus";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_TEXT_SLOT: &str =
    "__moliStorageBucketCachePutResponseStatusText";
const STORAGE_BUCKET_CACHE_PUT_RESPONSE_HEADERS_SLOT: &str =
    "__moliStorageBucketCachePutResponseHeaders";
pub(in crate::context_bootstrap) const NAVIGATOR_UA_DATA_BRAND_SLOT: &str =
    "__moliNavigatorUADataBrand";
const NAVIGATOR_UA_DATA_USER_AGENT_SLOT: &str = "__moliNavigatorUADataUserAgent";
pub(in crate::context_bootstrap) const MEDIA_DEVICES_BRAND_SLOT: &str = "__moliMediaDevicesBrand";
pub(in crate::context_bootstrap) const PERMISSIONS_BRAND_SLOT: &str = "__moliPermissionsBrand";
const NAVIGATOR_BATTERY_STATUS_BRAND_SLOT: &str = "__moliBatteryStatusBrand";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", own_to_string_tag = "CacheStorage")]
struct StorageBucketCacheStorageObjectDeclaration {
    #[webapi(slot = STORAGE_BUCKET_CACHE_STORAGE_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(
        method,
        callback = storage_bucket_cache_storage_match_callback,
        length = 1
    )]
    r#match: (),
    #[webapi(
        method,
        callback = storage_bucket_cache_storage_has_callback,
        length = 1
    )]
    has: (),
    #[webapi(
        method,
        callback = storage_bucket_cache_storage_open_callback,
        length = 1
    )]
    open: (),
    #[webapi(
        method,
        callback = storage_bucket_cache_storage_keys_callback,
        length = 0
    )]
    keys: (),
    #[webapi(
        method,
        callback = storage_bucket_cache_storage_delete_callback,
        length = 1
    )]
    delete: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", own_to_string_tag = "Cache")]
struct StorageBucketCacheObjectDeclaration {
    #[webapi(slot = STORAGE_BUCKET_CACHE_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(slot = STORAGE_BUCKET_CACHE_NAME_SLOT)]
    cache_name: String,
    #[webapi(slot = STORAGE_BUCKET_CACHE_ID_SLOT)]
    cache_id: String,
    #[webapi(method, callback = storage_bucket_cache_put_callback, length = 2)]
    put: (),
    #[webapi(method, callback = storage_bucket_cache_match_callback, length = 1)]
    r#match: (),
    #[webapi(method = "matchAll", callback = storage_bucket_cache_match_all_callback, length = 0)]
    match_all: (),
    #[webapi(method, callback = storage_bucket_cache_keys_callback, length = 0)]
    keys: (),
    #[webapi(method, callback = storage_bucket_cache_delete_callback, length = 1)]
    delete: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PermissionStatus")]
struct PermissionStatusObjectDeclaration {
    #[webapi(slot = PERMISSION_STATUS_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = PERMISSION_STATUS_NAME_SLOT)]
    name: String,

    #[webapi(slot = PERMISSION_STATUS_STATE_SLOT)]
    state: String,

    #[webapi(data_property, init = "null")]
    onchange: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PermissionDescriptor")]
struct PermissionDescriptorMembers {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MidiPermissionDescriptor")]
struct MidiPermissionDescriptorMembers {
    #[webidl(required)]
    name: String,

    #[webidl(default = false)]
    sysex: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct NavigatorUaBrandEntryDeclaration {
    #[webapi(data_property, enumerable)]
    brand: String,

    #[webapi(data_property, enumerable)]
    version: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "NavigatorUAData")]
struct NavigatorUaDataObjectDeclaration {
    #[webapi(slot = NAVIGATOR_UA_DATA_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = NAVIGATOR_UA_DATA_USER_AGENT_SLOT)]
    user_agent: String,

    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    platform: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct NavigatorUaDataSnapshotDeclaration {
    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    platform: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct NavigatorUaDataHighEntropySnapshotDeclaration {
    #[webapi(data_property, enumerable)]
    architecture: Option<String>,

    #[webapi(data_property, enumerable)]
    bitness: Option<String>,

    #[webapi(data_property, enumerable)]
    brands: Vec<NavigatorUaBrandEntryDeclaration>,

    #[webapi(data_property = "formFactors", enumerable)]
    form_factors: Option<Vec<String>>,

    #[webapi(data_property, enumerable)]
    full_version_list: Option<Vec<NavigatorUaBrandEntryDeclaration>>,

    #[webapi(data_property, enumerable)]
    mobile: bool,

    #[webapi(data_property, enumerable)]
    model: Option<String>,

    #[webapi(data_property, enumerable)]
    platform: String,

    #[webapi(data_property, enumerable)]
    platform_version: Option<String>,

    #[webapi(data_property, enumerable)]
    ua_full_version: Option<String>,

    #[webapi(data_property, enumerable)]
    wow64: Option<bool>,
}

#[derive(WebApiObject)]
#[webapi(interface = "StorageEstimate")]
struct StorageEstimateObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    quota: f64,

    #[webapi(data_property, enumerable)]
    usage: f64,

    #[webapi(data_property, enumerable)]
    usage_details: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct StorageUsageDetailsObjectDeclaration {
    #[webapi(data_property = "indexedDB", enumerable)]
    indexed_db: Option<f64>,
    #[webapi(data_property = "caches", enumerable)]
    caches: Option<f64>,
    #[webapi(data_property = "fileSystem", enumerable)]
    file_system: Option<f64>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties)]
struct NavigatorBatteryStatusObjectDeclaration {
    #[webapi(slot = NAVIGATOR_BATTERY_STATUS_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(data_property, enumerable)]
    charging: bool,
    #[webapi(data_property, enumerable)]
    charging_time: f64,
    #[webapi(data_property, enumerable)]
    discharging_time: f64,
    #[webapi(data_property, enumerable)]
    level: f64,

    #[webapi(data_property, enumerable, init = "null")]
    onchargingchange: (),

    #[webapi(data_property, enumerable, init = "null")]
    onchargingtimechange: (),

    #[webapi(data_property, enumerable, init = "null")]
    ondischargingtimechange: (),

    #[webapi(data_property, enumerable, init = "null")]
    onlevelchange: (),

    #[webapi(method, length = 2, callback = battery_event_target_noop_callback)]
    add_event_listener: (),

    #[webapi(method, length = 2, callback = battery_event_target_noop_callback)]
    remove_event_listener: (),

    #[webapi(method, length = 1, callback = battery_dispatch_event_callback)]
    dispatch_event: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "StorageBucket")]
struct StorageBucketObjectDeclaration {
    #[webapi(slot, name = STORAGE_BUCKET_BRAND_SLOT, constructor_default = true)]
    brand: bool,
}

#[derive(Debug)]
struct StorageBucketHandle {
    identity: StorageBucketIdentity,
    indexed_db_storage_key: String,
}

#[derive(Debug)]
struct StorageBucketCacheHandle {
    bucket: StorageBucketHandle,
    cache_name: String,
    cache_id: StorageBucketCacheId,
}

#[derive(Debug)]
struct CacheRequestInfo {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct CacheQueryOptions {
    ignore_search: bool,
    ignore_method: bool,
    ignore_vary: bool,
    cache_name: Option<String>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct StorageBucketCachePutPendingDataDeclaration<'scope> {
    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::PromiseResolver>,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_BUCKET_ORIGIN_SLOT)]
    bucket_origin: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_BUCKET_NAME_SLOT)]
    bucket_name: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_BUCKET_ID_SLOT)]
    bucket_id: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_BUCKET_STORAGE_KEY_SLOT)]
    bucket_storage_key: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_CACHE_NAME_SLOT)]
    cache_name: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_CACHE_ID_SLOT)]
    cache_id: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_REQUEST_KEY_SLOT)]
    request_key: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_REQUEST_METHOD_SLOT)]
    request_method: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_REQUEST_HEADERS_SLOT)]
    request_headers_json: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_TYPE_SLOT)]
    response_type: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_URL_SLOT)]
    response_url: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_REDIRECTED_SLOT)]
    response_redirected: bool,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_SLOT)]
    response_status: f64,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_TEXT_SLOT)]
    response_status_text: String,

    #[webapi(slot = STORAGE_BUCKET_CACHE_PUT_RESPONSE_HEADERS_SLOT)]
    response_headers_json: String,
}

enum StorageBucketCachedResponseMaterialization<'scope> {
    Ready(StorageBucketCachedResponse),
    Pending {
        head: crate::network_host::MaterializedResponseHead,
        promise: v8::Local<'scope, v8::Promise>,
    },
}

struct StorageManagerOwnerContext {
    storage_key: String,
    area_key: Option<String>,
    host_ptr: Option<*mut crate::native_bridge::JsContextHost>,
    popup_id: Option<u64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigator.vibrate")]
struct NavigatorVibrateArgs {
    #[webidl(required, with = navigator_vibration_pattern_arg)]
    pattern: Vec<u32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigatorUAData.getHighEntropyValues")]
struct NavigatorUaDataGetHighEntropyValuesArgs {
    #[webidl(required, with = navigator_ua_data_high_entropy_hints_arg)]
    hints: webidl::Sequence<webidl::DomString>,
}

fn navigator_ua_data_high_entropy_hints_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<webidl::Sequence<webidl::DomString>, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'getHighEntropyValues' on 'NavigatorUAData': 1 argument required, but only 0 present.",
        ));
    }
    webidl::convert(
        scope,
        args.get(index),
        webidl::Context::argument("NavigatorUAData.getHighEntropyValues", (index + 1) as usize),
    )
}

#[derive(Default)]
struct StorageBucketOpenOptions {
    expires: Option<f64>,
    durability: Option<StorageBucketDurability>,
    quota: Option<u64>,
    persisted: Option<bool>,
}

const MAX_STORAGE_BUCKET_QUOTA_BYTES: f64 = 9_007_199_254_740_991.0;

pub(in crate::context_bootstrap) fn navigator_send_beacon_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        crate::util::throw_type_error(scope, "Illegal invocation");
        return;
    }
    crate::network_host::navigator_send_beacon_callback(scope, args, rv);
}

pub(in crate::context_bootstrap) fn navigator_java_enabled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        crate::util::throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(v8::Boolean::new(scope, false).into());
}

fn navigator_vibration_pattern_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Vec<u32>, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'vibrate' on 'Navigator': 1 argument required, but only 0 present.",
        ));
    }
    let value = args.get(index);
    let context = webidl::Context::argument("Navigator.vibrate", (index + 1) as usize);
    if let Some(sequence) =
        webidl::convert_optional_sequence::<webidl::UnsignedLong>(scope, value, context, &())?
    {
        return Ok(sequence.0.into_iter().map(|value| value.0).collect());
    }
    webidl::convert::<webidl::UnsignedLong>(scope, value, context).map(|value| vec![value.0])
}

fn normalize_vibration_pattern(pattern: &mut Vec<u32>) {
    pattern.truncate(VIBRATION_PATTERN_LENGTH_MAX);
    for duration in pattern.iter_mut() {
        *duration = (*duration).min(VIBRATION_DURATION_MS_MAX);
    }
    if pattern.len().is_multiple_of(2) {
        pattern.pop();
    }
}

pub(in crate::context_bootstrap) fn navigator_vibrate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        crate::util::throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(mut parsed) = webidl::parse_args::<NavigatorVibrateArgs>(scope, &args) else {
        return;
    };
    if !current_protocol_user_gesture_activation(scope) {
        rv.set_bool(false);
        return;
    }
    normalize_vibration_pattern(&mut parsed.pattern);
    // Headless Moli has no vibration device. A valid, activated request
    // is therefore a successful no-op, as required for unavailable hardware.
    rv.set_bool(true);
}

pub(in crate::context_bootstrap) fn navigator_get_battery_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if !navigator_receiver_branded(scope, args.this()) {
        reject_type_error(scope, resolver, "Illegal invocation");
        rv.set(resolver.get_promise(scope).into());
        return;
    }
    let battery = NavigatorBatteryStatusObjectDeclaration::new(true, 0.0, f64::INFINITY, 1.0)
        .bind(scope)
        .expect("Navigator battery status declaration should bind");
    let _ = resolver.resolve(scope, battery.into());
    rv.set(resolver.get_promise(scope).into());
}

fn battery_event_target_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !battery_status_receiver_branded(scope, args.this()) {
        crate::util::throw_type_error(scope, "Illegal invocation");
    }
}

fn battery_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !battery_status_receiver_branded(scope, args.this()) {
        crate::util::throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(v8::Boolean::new(scope, true).into());
}

fn battery_status_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, NAVIGATOR_BATTERY_STATUS_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn navigator_permissions_query_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if !permissions_receiver_branded(scope, args.this()) {
        reject_type_error(scope, resolver, "Illegal invocation");
        rv.set(resolver.get_promise(scope).into());
        return;
    }
    let permission_name = match permission_query_name(scope, args.get(0)) {
        Ok(permission_name) => permission_name,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let state = unsafe { (&*host_ptr).permission_state(&permission_name) };
    let status = build_permission_status_object(scope, &permission_name, state).into();
    let _ = resolver.resolve(scope, status);
    rv.set(resolver.get_promise(scope).into());
}

fn permission_query_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<String, webidl::WebIdlError> {
    let descriptor_object = webidl::convert::<v8::Local<'s, v8::Object>>(
        scope,
        value,
        webidl::Context::argument("Permissions.query", 1),
    )?;
    let descriptor =
        webidl::parse_dictionary_object::<PermissionDescriptorMembers>(scope, descriptor_object)?;

    if descriptor.name == "midi" {
        // Permissions first converts the raw object to PermissionDescriptor,
        // then converts that same object to the permission-specific derived
        // dictionary. Both conversions are observable through getters.
        let midi = webidl::parse_dictionary_object::<MidiPermissionDescriptorMembers>(
            scope,
            descriptor_object,
        )?;
        let _ = (midi.name, midi.sysex);
    }

    Ok(descriptor.name)
}

fn permissions_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, PERMISSIONS_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn build_permission_status_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    state: &str,
) -> v8::Local<'s, v8::Object> {
    PermissionStatusObjectDeclaration::new(name.to_owned(), state.to_owned())
        .bind(scope)
        .expect("PermissionStatus declaration should bind")
}

pub(in crate::context_bootstrap) fn permission_status_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = get_private_value(scope, args.this(), PERMISSION_STATUS_NAME_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

pub(in crate::context_bootstrap) fn permission_status_state_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = get_private_value(scope, args.this(), PERMISSION_STATUS_STATE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

pub(in crate::context_bootstrap) fn navigator_storage_estimate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_manager_promise_resolver(scope, &args, &mut rv, "estimate") else {
        return;
    };
    let Some(context) = storage_manager_context_or_reject(scope, args.this(), resolver, "estimate")
    else {
        return;
    };
    let usage = storage_manager_usage_bytes(scope, &context);
    let estimate = build_storage_estimate_object_with_quota(
        scope,
        usage.total,
        usage.indexed_db,
        usage.cache_storage,
        usage.opfs,
        usage.quota,
    );
    let _ = resolver.resolve(scope, estimate.into());
}

pub(in crate::context_bootstrap) fn navigator_storage_persisted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_manager_promise_resolver(scope, &args, &mut rv, "persisted")
    else {
        return;
    };
    let Some(_context) =
        storage_manager_context_or_reject(scope, args.this(), resolver, "persisted")
    else {
        return;
    };
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, false).into());
}

pub(in crate::context_bootstrap) fn navigator_storage_persist_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_manager_promise_resolver(scope, &args, &mut rv, "persist") else {
        return;
    };
    let Some(_context) = storage_manager_context_or_reject(scope, args.this(), resolver, "persist")
    else {
        return;
    };
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, false).into());
}

pub(in crate::context_bootstrap) fn navigator_storage_get_directory_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_manager_promise_resolver(scope, &args, &mut rv, "getDirectory")
    else {
        return;
    };
    let Some(context) = storage_manager_owner_context(scope, args.this()) else {
        reject_storage_bucket_invalid_state(scope, resolver, "getDirectory");
        return;
    };
    if moli_storage_key::serialized_storage_key_has_opaque_origin(&context.storage_key) {
        reject_storage_bucket_security_error(scope, resolver, "StorageManager", "getDirectory");
        return;
    }
    let locator = moli_storage_service::StorageBucketLocator::default_bucket(context.storage_key);
    super::super::opfs::resolve_opfs_root(scope, resolver, locator);
}

fn storage_manager_promise_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    branded_promise_resolver(
        scope,
        args,
        rv,
        "StorageManager",
        method,
        STORAGE_MANAGER_BRAND_SLOT,
    )
}

fn storage_manager_context_or_reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) -> Option<StorageManagerOwnerContext> {
    let Some(context) = storage_manager_owner_context(scope, manager) else {
        let message = format!("StorageManager.{method} storage context is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    if moli_storage_key::serialized_storage_key_has_opaque_origin(&context.storage_key) {
        let message = format!("StorageManager.{method} is unavailable in opaque origins.");
        reject_type_error(scope, resolver, &message);
        return None;
    }
    Some(context)
}

fn storage_manager_owner_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<StorageManagerOwnerContext> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let context = {
            let host = unsafe { &mut *host_ptr };
            if let Some(handle) = storage_manager_child_handle(scope, manager) {
                host.storage_context_for_child_browsing_context(handle)?
            } else if let Some(popup_id) = storage_manager_popup_id(scope, manager)
                .or_else(|| crate::native_bridge::active_lightweight_popup_id(scope))
            {
                if storage_manager_popup_id(scope, manager).is_some()
                    && !host.lightweight_popup_is_open(popup_id)
                {
                    return None;
                }
                let storage_context = host.storage_context_for_lightweight_popup(popup_id)?;
                return Some(StorageManagerOwnerContext {
                    storage_key: storage_context.storage_key().serialized_storage_key(),
                    area_key: Some(storage_context.web_storage_area_key().to_owned()),
                    host_ptr: Some(host_ptr),
                    popup_id: Some(popup_id),
                });
            } else {
                host.top_document_storage_context()
            }
        };
        return Some(StorageManagerOwnerContext {
            storage_key: context.storage_key().serialized_storage_key(),
            area_key: Some(context.web_storage_area_key().to_owned()),
            host_ptr: Some(host_ptr),
            popup_id: None,
        });
    }
    let storage_key = crate::worker::worker_storage_key(scope)?;
    let storage_key = storage_key.serialized_storage_key();
    Some(StorageManagerOwnerContext {
        storage_key,
        area_key: None,
        host_ptr: None,
        popup_id: None,
    })
}

fn storage_manager_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, manager, STORAGE_MANAGER_CHILD_HANDLE_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

fn storage_manager_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    private_popup_id(scope, manager, STORAGE_MANAGER_POPUP_ID_SLOT)
}

fn storage_manager_usage_bytes(
    scope: &mut v8::PinScope<'_, '_>,
    context: &StorageManagerOwnerContext,
) -> StorageUsageSnapshot {
    let web_storage_usage = context
        .host_ptr
        .zip(context.area_key.as_deref())
        .map(|(host_ptr, area_key)| {
            let host = unsafe { &*host_ptr };
            let local_usage = {
                let store = host.web_storage_store();
                store.lock().usage_bytes(area_key)
            };
            let session_usage = if let Some(popup_id) = context.popup_id {
                host.lightweight_popup_session_storage_store(popup_id)
                    .map(|store| store.lock().usage_bytes(area_key))
                    .unwrap_or(0)
            } else {
                let store = host.session_storage_store();
                store.lock().usage_bytes(area_key)
            };
            local_usage.saturating_add(session_usage) as u64
        })
        .unwrap_or(0);
    let locator = moli_storage_service::StorageBucketLocator::default_bucket(&context.storage_key);
    let bucket_usage = storage_bucket_quota_owner_for_locator(scope, &locator)
        .and_then(|owner| owner.usage_snapshot().ok());
    let (quota, indexed_db_usage, cache_storage_usage, opfs_usage) = bucket_usage
        .map(|usage| {
            (
                usage.quota,
                usage.indexed_db,
                usage.cache_storage,
                usage.opfs,
            )
        })
        .unwrap_or_else(|| {
            (
                crate::context_bootstrap::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES,
                indexed_db_usage_bytes_for_storage_key(scope, &context.storage_key),
                0,
                0,
            )
        });
    StorageUsageSnapshot {
        total: web_storage_usage
            .saturating_add(indexed_db_usage)
            .saturating_add(cache_storage_usage)
            .saturating_add(opfs_usage),
        quota,
        indexed_db: indexed_db_usage,
        cache_storage: cache_storage_usage,
        opfs: opfs_usage,
    }
}

pub(in crate::context_bootstrap) fn storage_bucket_manager_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_manager_resolver(scope, &args, &mut rv, "open") else {
        return;
    };
    let Some(name) = required_dom_string_argument(scope, &args, 0, "open", "StorageBucketManager")
    else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open requires a bucket name.",
        );
        return;
    };
    if !valid_storage_bucket_name(&name) {
        let message = format!("The bucket name '{name}' is not a valid name.");
        reject_type_error(scope, resolver, &message);
        return;
    }
    let Some(options) = storage_bucket_open_options(scope, &args, resolver) else {
        return;
    };
    let now_ms = storage_bucket_now_ms();
    if options.expires.is_some_and(|expires| expires <= now_ms) {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open options.expires must be in the future.",
        );
        return;
    }
    let Some(storage_key) =
        storage_bucket_manager_storage_key_or_reject(scope, args.this(), resolver, "open")
    else {
        return;
    };
    let expired = with_storage_bucket_store_entry(scope, |store| {
        store.delete_bucket_if_expired(&storage_key, &name, now_ms)
    });
    match expired {
        Some(Ok(Some(cleanup))) => {
            if let Err(error) = complete_storage_bucket_deletion(scope, &cleanup) {
                reject_type_error(scope, resolver, &error.to_string());
                return;
            }
        }
        Some(Ok(None)) => {}
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucketManager.open storage bucket store is unavailable.",
            );
            return;
        }
    }
    let opened = with_storage_bucket_store_entry(scope, |store| {
        store.open_bucket_with_options(
            &storage_key,
            &name,
            options.expires,
            options.durability,
            options.quota,
            options.persisted,
        )
    });
    let identity = match opened {
        Some(Ok(opened)) => opened,
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucketManager.open storage bucket store is unavailable.",
            );
            return;
        }
    };
    let bucket = build_storage_bucket_object(scope, identity);
    let _ = resolver.resolve(scope, bucket.into());
}

pub(in crate::context_bootstrap) fn storage_bucket_manager_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_manager_resolver(scope, &args, &mut rv, "keys") else {
        return;
    };
    let Some(storage_key) =
        storage_bucket_manager_storage_key_or_reject(scope, args.this(), resolver, "keys")
    else {
        return;
    };
    let expired = with_storage_bucket_store_entry(scope, |store| {
        store.delete_expired_buckets(&storage_key, storage_bucket_now_ms())
    });
    match expired {
        Some(Ok(expired)) => {
            for cleanup in expired {
                if let Err(error) = complete_storage_bucket_deletion(scope, &cleanup) {
                    reject_type_error(scope, resolver, &error.to_string());
                    return;
                }
            }
        }
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucketManager.keys storage bucket store is unavailable.",
            );
            return;
        }
    }
    let Some(names) = with_storage_bucket_store_entry(scope, |store| store.keys(&storage_key))
    else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.keys storage bucket store is unavailable.",
        );
        return;
    };
    let array = strings_to_array(scope, &names);
    let _ = resolver.resolve(scope, array.into());
}

pub(in crate::context_bootstrap) fn storage_bucket_manager_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_manager_resolver(scope, &args, &mut rv, "delete") else {
        return;
    };
    let Some(name) =
        required_dom_string_argument(scope, &args, 0, "delete", "StorageBucketManager")
    else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.delete requires a bucket name.",
        );
        return;
    };
    if !valid_storage_bucket_name(&name) {
        let message = format!("The bucket name '{name}' is not a valid name.");
        reject_type_error(scope, resolver, &message);
        return;
    }
    let Some(storage_key) =
        storage_bucket_manager_storage_key_or_reject(scope, args.this(), resolver, "delete")
    else {
        return;
    };
    let deleted =
        with_storage_bucket_store_entry(scope, |store| store.delete_bucket(&storage_key, &name));
    let cleanup = match deleted {
        Some(Ok(cleanup)) => cleanup,
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucketManager.delete storage bucket store is unavailable.",
            );
            return;
        }
    };
    if let Some(cleanup) = cleanup
        && let Err(error) = complete_storage_bucket_deletion(scope, &cleanup)
    {
        reject_type_error(scope, resolver, &error.to_string());
        return;
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn storage_bucket_manager_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = branded_promise_resolver(
        scope,
        args,
        rv,
        "StorageBucketManager",
        method,
        STORAGE_BUCKET_MANAGER_BRAND_SLOT,
    )?;
    if !storage_bucket_manager_owner_is_live(scope, args.this()) {
        reject_illegal_invocation(scope, resolver, "StorageBucketManager", method);
        return None;
    }
    Some(resolver)
}

fn storage_bucket_manager_storage_key_or_reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) -> Option<String> {
    let storage_key = storage_bucket_manager_storage_key(scope, manager);
    let Some(storage_key) = storage_key else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager storage bucket storage key is unavailable.",
        );
        return None;
    };
    if !storage_bucket_origin_allows_storage(&storage_key) {
        reject_storage_bucket_security_error(scope, resolver, "StorageBucketManager", method);
        return None;
    }
    Some(storage_key)
}

fn storage_bucket_manager_storage_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let context = if let Some(handle) = storage_bucket_manager_child_handle(scope, manager) {
            host.storage_context_for_child_browsing_context(handle)?
        } else if let Some(popup_id) = storage_bucket_manager_popup_id(scope, manager)
            .or_else(|| crate::native_bridge::active_lightweight_popup_id(scope))
        {
            host.storage_context_for_lightweight_popup(popup_id)?
        } else {
            host.top_document_storage_context()
        };
        return Some(context.storage_key().serialized_storage_key());
    }
    current_storage_bucket_storage_key(scope)
}

fn storage_bucket_manager_owner_is_live<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(popup_id) = storage_bucket_manager_popup_id(scope, manager) {
        return context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
            // SAFETY: the current V8 context owns a bridge ref for this host pointer.
            // Lightweight popups still share the opener's concrete realm, so
            // their explicit popup owner remains authoritative until P2 gives
            // them an independent execution context.
            unsafe { (&*host_ptr).lightweight_popup_is_open(popup_id) }
        });
    }
    if !storage_bucket_receiver_execution_context_is_live(scope, manager) {
        return false;
    }
    if let Some(handle) = storage_bucket_manager_child_handle(scope, manager) {
        return context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
            // SAFETY: context_host_ptr_from_global_bridge returns the live
            // JsContextHost pointer owned by the current V8 context.
            unsafe { (&*host_ptr).child_browsing_context_is_live(handle) }
        });
    }
    true
}

fn storage_bucket_manager_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, manager, STORAGE_BUCKET_MANAGER_CHILD_HANDLE_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

fn storage_bucket_manager_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    private_popup_id(scope, manager, STORAGE_BUCKET_MANAGER_POPUP_ID_SLOT)
}

fn private_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<u64> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (popup_id, lossless) = value.u64_value();
            (lossless && popup_id != 0).then_some(popup_id)
        })
}

pub(in crate::context_bootstrap) fn storage_bucket_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !storage_bucket_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), STORAGE_BUCKET_NAME_SLOT)
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn storage_bucket_indexed_db_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !storage_bucket_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handle) = storage_bucket_handle(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let value = scoped_storage_bucket_indexed_db_factory(scope, &handle.identity)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn storage_bucket_caches_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !storage_bucket_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(value) = get_private_value(scope, args.this(), STORAGE_BUCKET_CACHES_OBJECT_SLOT) {
        rv.set(value);
        return;
    }
    let Some(handle) = storage_bucket_handle(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let cache_storage = build_storage_bucket_cache_storage_object(scope, &handle);
    set_private_value(
        scope,
        args.this(),
        STORAGE_BUCKET_CACHES_OBJECT_SLOT,
        cache_storage.into(),
    );
    rv.set(cache_storage.into());
}

pub(in crate::context_bootstrap) fn global_caches_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), GLOBAL_CACHE_STORAGE_SLOT) {
        rv.set(value);
        return;
    }
    let Some(storage_key) = current_storage_bucket_storage_key(scope) else {
        rv.set_undefined();
        return;
    };
    if !storage_bucket_origin_allows_storage(&storage_key) {
        rv.set_undefined();
        return;
    }
    let opened = with_storage_bucket_store_entry(scope, |store| {
        store.open_bucket(&storage_key, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
    });
    let identity = match opened {
        Some(Ok(opened)) => opened,
        Some(Err(_)) | None => {
            rv.set_undefined();
            return;
        }
    };
    let handle = StorageBucketHandle {
        indexed_db_storage_key: identity.indexed_db_storage_key(),
        identity,
    };
    let cache_storage = build_storage_bucket_cache_storage_object(scope, &handle);
    set_private_value(
        scope,
        args.this(),
        GLOBAL_CACHE_STORAGE_SLOT,
        cache_storage.into(),
    );
    rv.set(cache_storage.into());
}

pub(in crate::context_bootstrap) fn storage_bucket_persist_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "persist") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "persist",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let Some(persisted) = storage_bucket_persisted_value(scope, &handle, resolver, "persist")
    else {
        return;
    };
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, persisted).into());
}

pub(in crate::context_bootstrap) fn storage_bucket_persisted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "persisted") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "persisted",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let Some(persisted) = storage_bucket_persisted_value(scope, &handle, resolver, "persisted")
    else {
        return;
    };
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, persisted).into());
}

fn storage_bucket_persisted_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &StorageBucketHandle,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) -> Option<bool> {
    let persisted = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_persisted_for_identity(&handle.identity)
    });
    match persisted {
        Some(Some(persisted)) => Some(persisted),
        Some(None) => {
            reject_storage_bucket_unknown_error(scope, resolver, method);
            None
        }
        None => {
            let message = format!("StorageBucket.{method} storage bucket store is unavailable.");
            reject_type_error(scope, resolver, &message);
            None
        }
    }
}

pub(in crate::context_bootstrap) fn storage_bucket_estimate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "estimate") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "estimate",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let indexed_db_usage = storage_bucket_indexed_db_usage_bytes(scope, &handle);
    let cache_storage_usage = storage_bucket_cache_storage_usage_bytes(scope, &handle);
    let opfs_owner = with_storage_bucket_store_entry(scope, |store| {
        (
            store.bucket_locator_for_identity(&handle.identity),
            store.storage_service(),
        )
    });
    let opfs_usage = match opfs_owner {
        Some((Some(locator), storage_service)) => match storage_service.opfs_usage(&locator) {
            Ok(usage) => usage,
            Err(error) => {
                reject_type_error(scope, resolver, &error.to_string());
                return;
            }
        },
        Some((None, _)) => {
            reject_storage_bucket_unknown_error(scope, resolver, "estimate");
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.estimate storage bucket store is unavailable.",
            );
            return;
        }
    };
    let usage = indexed_db_usage
        .saturating_add(cache_storage_usage)
        .saturating_add(opfs_usage);
    let quota = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_quota_for_identity(&handle.identity)
    });
    let quota = match quota {
        Some(Some(quota)) => {
            quota.unwrap_or(crate::context_bootstrap::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES)
        }
        Some(None) => {
            reject_storage_bucket_unknown_error(scope, resolver, "estimate");
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.estimate storage bucket store is unavailable.",
            );
            return;
        }
    };
    let estimate = build_storage_estimate_object_with_quota(
        scope,
        usage,
        indexed_db_usage,
        cache_storage_usage,
        opfs_usage,
        quota,
    );
    let _ = resolver.resolve(scope, estimate.into());
}

fn storage_bucket_cache_storage_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_storage_resolver(scope, &args, &mut rv, "open")
    else {
        return;
    };
    let Some(cache_name) = required_dom_string_argument(scope, &args, 0, "open", "CacheStorage")
    else {
        reject_type_error(scope, resolver, "CacheStorage.open requires a cache name.");
        return;
    };
    let Some(handle) =
        storage_bucket_cache_storage_live_handle(scope, args.this(), resolver, "open")
    else {
        return;
    };
    let opened = with_storage_bucket_store_entry(scope, |store| {
        store.open_cache_handle_for_identity(&handle.identity, &cache_name)
    });
    match opened {
        Some(Ok(Some(cache_id))) => {
            let cache = build_storage_bucket_cache_object(scope, &handle, &cache_name, cache_id);
            let _ = resolver.resolve(scope, cache.into());
        }
        Some(Ok(None)) => reject_storage_bucket_unknown_error(scope, resolver, "caches.open"),
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
        }
        None => reject_type_error(
            scope,
            resolver,
            "CacheStorage.open storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_storage_match_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_storage_resolver(scope, &args, &mut rv, "match")
    else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(Some(request)) => request,
        Ok(None) => {
            reject_type_error(
                scope,
                resolver,
                "CacheStorage.match requires a request key.",
            );
            return;
        }
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    let Some(options) = cache_query_options(scope, &args, 1) else {
        reject_type_error(scope, resolver, "CacheStorage.match options are invalid.");
        return;
    };
    let query = storage_bucket_cache_query(request, &options);
    let Some(handle) =
        storage_bucket_cache_storage_live_handle(scope, args.this(), resolver, "match")
    else {
        return;
    };
    let matched = with_storage_bucket_store_entry(scope, |store| {
        let cache_names = match options.cache_name.as_ref() {
            Some(cache_name) => vec![cache_name.clone()],
            None => store.cache_names_for_identity(&handle.identity)?,
        };
        for cache_name in cache_names {
            if let Some(response) = store
                .match_cache_entries_for_identity(&handle.identity, &cache_name, &query)?
                .into_iter()
                .next()
                .map(|entry| entry.response)
            {
                return Some(Some(response));
            }
        }
        Some(None)
    });
    match matched {
        Some(Some(Some(response))) => {
            let Some(response) = build_storage_bucket_cached_response_object(scope, response)
            else {
                reject_type_error(
                    scope,
                    resolver,
                    "CacheStorage.match failed to build a Response.",
                );
                return;
            };
            let _ = resolver.resolve(scope, response.into());
        }
        Some(Some(None)) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "caches.match"),
        None => reject_type_error(
            scope,
            resolver,
            "CacheStorage.match storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_storage_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_storage_resolver(scope, &args, &mut rv, "has") else {
        return;
    };
    let Some(cache_name) = required_dom_string_argument(scope, &args, 0, "has", "CacheStorage")
    else {
        reject_type_error(scope, resolver, "CacheStorage.has requires a cache name.");
        return;
    };
    let Some(handle) =
        storage_bucket_cache_storage_live_handle(scope, args.this(), resolver, "has")
    else {
        return;
    };
    let names = with_storage_bucket_store_entry(scope, |store| {
        store.cache_names_for_identity(&handle.identity)
    });
    match names {
        Some(Some(names)) => {
            let exists = names.iter().any(|name| name == &cache_name);
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, exists).into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "caches.has"),
        None => reject_type_error(
            scope,
            resolver,
            "CacheStorage.has storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_storage_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_storage_resolver(scope, &args, &mut rv, "keys")
    else {
        return;
    };
    let Some(handle) =
        storage_bucket_cache_storage_live_handle(scope, args.this(), resolver, "keys")
    else {
        return;
    };
    let names = with_storage_bucket_store_entry(scope, |store| {
        store.cache_names_for_identity(&handle.identity)
    });
    match names {
        Some(Some(names)) => {
            let array = strings_to_array(scope, &names);
            let _ = resolver.resolve(scope, array.into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "caches.keys"),
        None => reject_type_error(
            scope,
            resolver,
            "CacheStorage.keys storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_storage_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_storage_resolver(scope, &args, &mut rv, "delete")
    else {
        return;
    };
    let Some(cache_name) = required_dom_string_argument(scope, &args, 0, "delete", "CacheStorage")
    else {
        reject_type_error(
            scope,
            resolver,
            "CacheStorage.delete requires a cache name.",
        );
        return;
    };
    let Some(handle) =
        storage_bucket_cache_storage_live_handle(scope, args.this(), resolver, "delete")
    else {
        return;
    };
    let deleted = with_storage_bucket_store_entry(scope, |store| {
        store.delete_cache_for_identity(&handle.identity, &cache_name)
    });
    match deleted {
        Some(Ok(Some(deleted))) => {
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, deleted).into());
        }
        Some(Ok(None)) => reject_storage_bucket_unknown_error(scope, resolver, "caches.delete"),
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
        }
        None => reject_type_error(
            scope,
            resolver,
            "CacheStorage.delete storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_put_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, &mut rv, "put") else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(Some(request)) => request,
        Ok(None) => {
            reject_type_error(scope, resolver, "Cache.put requires a request key.");
            return;
        }
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    if !request.method.eq_ignore_ascii_case("GET") {
        reject_type_error(scope, resolver, "Cache.put only accepts GET requests.");
        return;
    }
    if args.length() <= 1 || args.get(1).is_null_or_undefined() {
        reject_type_error(scope, resolver, "Cache.put requires a response.");
        return;
    }
    let Some(response) = storage_bucket_cached_response_from_value(scope, args.get(1), resolver)
    else {
        return;
    };
    let Some(handle) = storage_bucket_cache_live_handle(scope, args.this(), resolver, "put") else {
        return;
    };
    match response {
        StorageBucketCachedResponseMaterialization::Ready(response) => {
            storage_bucket_cache_put_store_response(scope, resolver, handle, request, response);
        }
        StorageBucketCachedResponseMaterialization::Pending { head, promise } => {
            storage_bucket_cache_put_pending_body(scope, resolver, handle, request, head, promise);
        }
    }
}

fn storage_bucket_cache_put_store_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    handle: StorageBucketCacheHandle,
    request: CacheRequestInfo,
    response: StorageBucketCachedResponse,
) {
    let usage_bytes = cache_entry_usage_bytes(&request, &response);
    let locator = if handle.bucket.identity.name() == IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME {
        Some(moli_storage_service::StorageBucketLocator::default_bucket(
            handle.bucket.identity.storage_key(),
        ))
    } else {
        with_storage_bucket_store_entry(scope, |store| {
            store.bucket_locator_for_identity(&handle.bucket.identity)
        })
        .flatten()
    };
    let Some(quota_owner) = locator
        .as_ref()
        .and_then(|locator| storage_bucket_quota_owner_for_locator(scope, locator))
    else {
        reject_storage_bucket_unknown_error(scope, resolver, "cache.put");
        return;
    };
    let _quota_reservation = quota_owner.reserve_commit();
    let (_, non_cache_usage) = match quota_owner.quota_and_non_cache_usage() {
        Ok(usage) => usage,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    let stored = with_storage_bucket_store_entry(scope, |store| {
        store.put_cache_entry_with_request_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
            &request.url,
            StorageBucketCachedRequest {
                method: request.method,
                headers: request.headers,
            },
            response,
            usage_bytes,
            non_cache_usage,
        )
    });
    match stored {
        Some(Ok(StorageBucketCachePutOutcome::Stored)) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Some(Ok(StorageBucketCachePutOutcome::Stale)) => {
            reject_storage_bucket_unknown_error(scope, resolver, "cache.put");
        }
        Some(Ok(StorageBucketCachePutOutcome::QuotaExceeded { quota, requested })) => {
            reject_storage_bucket_quota_exceeded(scope, resolver, quota, requested);
        }
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
        }
        None => reject_type_error(
            scope,
            resolver,
            "Cache.put storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_put_pending_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    handle: StorageBucketCacheHandle,
    request: CacheRequestInfo,
    head: crate::network_host::MaterializedResponseHead,
    promise: v8::Local<'s, v8::Promise>,
) {
    let data = StorageBucketCachePutPendingDataDeclaration {
        resolver,
        bucket_origin: handle.bucket.identity.storage_key().to_owned(),
        bucket_name: handle.bucket.identity.name().to_owned(),
        bucket_id: handle.bucket.identity.bucket_id().get().to_string(),
        bucket_storage_key: handle.bucket.indexed_db_storage_key,
        cache_name: handle.cache_name,
        cache_id: handle.cache_id.get().to_string(),
        request_key: request.url,
        request_method: request.method,
        request_headers_json: serde_json::to_string(&request.headers)
            .unwrap_or_else(|_| "[]".to_owned()),
        response_type: head.response_type,
        response_url: head
            .final_url
            .map(|url| url.to_string())
            .unwrap_or_default(),
        response_redirected: head.redirected,
        response_status: f64::from(head.status),
        response_status_text: head.status_text,
        response_headers_json: serde_json::to_string(&head.headers)
            .unwrap_or_else(|_| "[]".to_owned()),
    }
    .bind(scope)
    .expect("Cache.put pending body data should bind");
    let Some(on_fulfilled) =
        v8::Function::builder(storage_bucket_cache_put_body_fulfilled_callback)
            .data(data.into())
            .build(scope)
    else {
        reject_type_error(
            scope,
            resolver,
            "Cache.put failed to attach response body reactions.",
        );
        return;
    };
    let Some(on_rejected) = v8::Function::builder(storage_bucket_cache_put_body_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        reject_type_error(
            scope,
            resolver,
            "Cache.put failed to attach response body reactions.",
        );
        return;
    };
    if promise.then2(scope, on_fulfilled, on_rejected).is_none() {
        reject_type_error(
            scope,
            resolver,
            "Cache.put failed to attach response body reactions.",
        );
    }
}

fn storage_bucket_cache_put_body_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if !storage_bucket_receiver_execution_context_is_live(scope, data) {
        return;
    }
    let Some(pending) = storage_bucket_cache_put_pending_data(scope, args.data()) else {
        return;
    };
    let body = match crate::network_host::materialized_body_bytes_from_value(scope, args.get(0)) {
        Ok(body) => body,
        Err(error) => {
            reject_type_error(scope, pending.resolver, &format!("Cache.put {error}"));
            return;
        }
    };
    let response = StorageBucketCachedResponse {
        response_type: pending.response_type,
        url: pending.response_url,
        redirected: pending.response_redirected,
        status: pending.response_status,
        status_text: pending.response_status_text,
        headers: pending.response_headers,
        body,
    };
    storage_bucket_cache_put_store_response(
        scope,
        pending.resolver,
        pending.handle,
        pending.request,
        response,
    );
}

fn storage_bucket_cache_put_body_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(pending) = storage_bucket_cache_put_pending_data(scope, args.data()) else {
        return;
    };
    let reason = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown rejection".to_owned());
    reject_type_error(
        scope,
        pending.resolver,
        &format!("Cache.put failed to materialize Response body: {reason}"),
    );
}

struct StorageBucketCachePutPendingData<'scope> {
    resolver: v8::Local<'scope, v8::PromiseResolver>,
    handle: StorageBucketCacheHandle,
    request: CacheRequestInfo,
    response_type: String,
    response_url: String,
    response_redirected: bool,
    response_status: u16,
    response_status_text: String,
    response_headers: Vec<(String, String)>,
}

fn storage_bucket_cache_put_pending_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<StorageBucketCachePutPendingData<'s>> {
    let data = v8::Local::<v8::Object>::try_from(value).ok()?;
    let resolver = get_private_value(scope, data, STORAGE_BUCKET_CACHE_PUT_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })?;
    let bucket_origin =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_BUCKET_ORIGIN_SLOT)?;
    let bucket_name = data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_BUCKET_NAME_SLOT)?;
    let bucket_id = data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_BUCKET_ID_SLOT)?
        .parse::<u64>()
        .ok()
        .and_then(moli_storage_service::StorageBucketId::new)?;
    let bucket_storage_key = data_private_string(
        scope,
        data,
        STORAGE_BUCKET_CACHE_PUT_BUCKET_STORAGE_KEY_SLOT,
    )?;
    let cache_name = data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_CACHE_NAME_SLOT)?;
    let cache_id = data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_CACHE_ID_SLOT)?
        .parse::<u64>()
        .ok()
        .map(StorageBucketCacheId::from_raw)?;
    let request_key = data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_REQUEST_KEY_SLOT)?;
    let request_method =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_REQUEST_METHOD_SLOT)?;
    let request_headers_json =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_REQUEST_HEADERS_SLOT)?;
    let request_headers =
        serde_json::from_str::<Vec<(String, String)>>(&request_headers_json).unwrap_or_default();
    let response_type =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_RESPONSE_TYPE_SLOT)?;
    let response_url =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_RESPONSE_URL_SLOT)?;
    let response_redirected = data_private_bool(
        scope,
        data,
        STORAGE_BUCKET_CACHE_PUT_RESPONSE_REDIRECTED_SLOT,
    );
    let response_status =
        get_private_value(scope, data, STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_SLOT)
            .and_then(|value| value.number_value(scope))
            .map(|value| value as u16)?;
    let response_status_text = data_private_string(
        scope,
        data,
        STORAGE_BUCKET_CACHE_PUT_RESPONSE_STATUS_TEXT_SLOT,
    )?;
    let response_headers_json =
        data_private_string(scope, data, STORAGE_BUCKET_CACHE_PUT_RESPONSE_HEADERS_SLOT)?;
    let response_headers =
        serde_json::from_str::<Vec<(String, String)>>(&response_headers_json).unwrap_or_default();
    let bucket = StorageBucketHandle {
        identity: StorageBucketIdentity::new(&bucket_origin, &bucket_name, bucket_id),
        indexed_db_storage_key: bucket_storage_key,
    };
    Some(StorageBucketCachePutPendingData {
        resolver,
        handle: StorageBucketCacheHandle {
            bucket,
            cache_name,
            cache_id,
        },
        request: CacheRequestInfo {
            url: request_key,
            method: request_method,
            headers: request_headers,
        },
        response_type,
        response_url,
        response_redirected,
        response_status,
        response_status_text,
        response_headers,
    })
}

fn data_private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, data, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn data_private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, data, slot).is_some_and(|value| value.boolean_value(scope))
}

fn storage_bucket_cache_match_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, &mut rv, "match") else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(Some(request)) => request,
        Ok(None) => {
            reject_type_error(scope, resolver, "Cache.match requires a request key.");
            return;
        }
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    let Some(options) = cache_query_options(scope, &args, 1) else {
        reject_type_error(scope, resolver, "Cache.match options are invalid.");
        return;
    };
    let query = storage_bucket_cache_query(request, &options);
    let Some(handle) = storage_bucket_cache_live_handle(scope, args.this(), resolver, "match")
    else {
        return;
    };
    let matched = with_storage_bucket_store_entry(scope, |store| {
        store
            .match_cache_entries_for_handle_and_identity(
                &handle.bucket.identity,
                &handle.cache_name,
                handle.cache_id,
                &query,
            )
            .map(|matches| matches.into_iter().next().map(|entry| entry.response))
    });
    match matched {
        Some(Some(Some(response))) => {
            let Some(response) = build_storage_bucket_cached_response_object(scope, response)
            else {
                reject_type_error(scope, resolver, "Cache.match failed to build a Response.");
                return;
            };
            let _ = resolver.resolve(scope, response.into());
        }
        Some(Some(None)) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "cache.match"),
        None => reject_type_error(
            scope,
            resolver,
            "Cache.match storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_match_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, &mut rv, "matchAll") else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(request) => request,
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    let Some(options) = cache_query_options(scope, &args, 1) else {
        reject_type_error(scope, resolver, "Cache.matchAll options are invalid.");
        return;
    };
    let query = request.map(|request| storage_bucket_cache_query(request, &options));
    let Some(handle) = storage_bucket_cache_live_handle(scope, args.this(), resolver, "matchAll")
    else {
        return;
    };
    let matched = with_storage_bucket_store_entry(scope, |store| match query.as_ref() {
        Some(query) => store.match_cache_entries_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
            query,
        ),
        None => store.cache_entries_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
        ),
    });
    match matched {
        Some(Some(matches)) => {
            let responses = v8::Array::new(scope, matches.len() as i32);
            for (index, matched) in matches.into_iter().enumerate() {
                let Some(response) =
                    build_storage_bucket_cached_response_object(scope, matched.response)
                else {
                    reject_type_error(
                        scope,
                        resolver,
                        "Cache.matchAll failed to build a Response.",
                    );
                    return;
                };
                if responses.set_index(scope, index as u32, response.into()) != Some(true) {
                    reject_type_error(
                        scope,
                        resolver,
                        "Cache.matchAll failed to build its result.",
                    );
                    return;
                }
            }
            let _ = resolver.resolve(scope, responses.into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "cache.matchAll"),
        None => reject_type_error(
            scope,
            resolver,
            "Cache.matchAll storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, &mut rv, "keys") else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(request) => request,
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    let Some(options) = cache_query_options(scope, &args, 1) else {
        reject_type_error(scope, resolver, "Cache.keys options are invalid.");
        return;
    };
    let query = request.map(|request| storage_bucket_cache_query(request, &options));
    let Some(handle) = storage_bucket_cache_live_handle(scope, args.this(), resolver, "keys")
    else {
        return;
    };
    let entries = with_storage_bucket_store_entry(scope, |store| match query.as_ref() {
        Some(query) => store.match_cache_entries_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
            query,
        ),
        None => store.cache_entries_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
        ),
    });
    match entries {
        Some(Some(entries)) => {
            let Some(requests) = cache_entries_to_request_array(scope, &entries) else {
                reject_type_error(scope, resolver, "Cache.keys failed to build a Request.");
                return;
            };
            let _ = resolver.resolve(scope, requests.into());
        }
        Some(None) => reject_storage_bucket_unknown_error(scope, resolver, "cache.keys"),
        None => reject_type_error(
            scope,
            resolver,
            "Cache.keys storage bucket store is unavailable.",
        ),
    }
}

fn storage_bucket_cache_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, &mut rv, "delete") else {
        return;
    };
    let request = match cache_request_info_argument(scope, &args, 0) {
        Ok(Some(request)) => request,
        Ok(None) => {
            reject_type_error(scope, resolver, "Cache.delete requires a request key.");
            return;
        }
        Err(error) => {
            reject_type_error(scope, resolver, &error);
            return;
        }
    };
    let Some(options) = cache_query_options(scope, &args, 1) else {
        reject_type_error(scope, resolver, "Cache.delete options are invalid.");
        return;
    };
    let query = storage_bucket_cache_query(request, &options);
    let Some(handle) = storage_bucket_cache_live_handle(scope, args.this(), resolver, "delete")
    else {
        return;
    };
    let deleted = with_storage_bucket_store_entry(scope, |store| {
        store.delete_cache_entries_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
            &query,
        )
    });
    match deleted {
        Some(Ok(Some(deleted))) => {
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, deleted).into());
        }
        Some(Ok(None)) => reject_storage_bucket_unknown_error(scope, resolver, "cache.delete"),
        Some(Err(error)) => reject_type_error(scope, resolver, &error.to_string()),
        None => reject_type_error(
            scope,
            resolver,
            "Cache.delete storage bucket store is unavailable.",
        ),
    }
}

pub(in crate::context_bootstrap) fn storage_bucket_durability_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "durability") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "durability",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let durability = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_durability_for_identity(&handle.identity)
    });
    let value = match durability {
        Some(Some(durability)) => v8str(scope, durability.as_str()).into(),
        Some(None) => {
            reject_storage_bucket_unknown_error(scope, resolver, "durability");
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.durability storage bucket store is unavailable.",
            );
            return;
        }
    };
    let _ = resolver.resolve(scope, value);
}

pub(in crate::context_bootstrap) fn storage_bucket_set_expires_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "setExpires") else {
        return;
    };
    let expires_arg = args.get(0);
    let expires = if expires_arg.is_null() {
        None
    } else {
        let expires = expires_arg.number_value(scope).unwrap_or(f64::NAN);
        if !expires.is_finite() {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.setExpires requires a finite timestamp.",
            );
            return;
        }
        Some(expires)
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "setExpires",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let result = with_storage_bucket_store_entry(scope, |store| {
        store.set_bucket_expires_for_identity(&handle.identity, expires)
    });
    match result {
        Some(Ok(true)) => {}
        Some(Ok(false)) => {
            reject_storage_bucket_unknown_error(scope, resolver, "setExpires");
            return;
        }
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.setExpires storage bucket store is unavailable.",
            );
            return;
        }
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

pub(in crate::context_bootstrap) fn storage_bucket_expires_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "expires") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "expires",
        StorageBucketStaleError::Unknown,
    ) else {
        return;
    };
    let expires = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_expires_for_identity(&handle.identity)
    });
    let value = match expires {
        Some(Some(expires)) => expires
            .map(|expires| v8::Number::new(scope, expires).into())
            .unwrap_or_else(|| v8::null(scope).into()),
        Some(None) => {
            reject_storage_bucket_unknown_error(scope, resolver, "expires");
            return;
        }
        None => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucket.expires storage bucket store is unavailable.",
            );
            return;
        }
    };
    let _ = resolver.resolve(scope, value);
}

pub(in crate::context_bootstrap) fn storage_bucket_get_directory_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = storage_bucket_resolver(scope, &args, &mut rv, "getDirectory") else {
        return;
    };
    let Some(handle) = storage_bucket_live_handle(
        scope,
        args.this(),
        resolver,
        "getDirectory",
        StorageBucketStaleError::InvalidState,
    ) else {
        return;
    };
    let locator = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_locator_for_identity(&handle.identity)
    })
    .flatten();
    let Some(locator) = locator else {
        reject_storage_bucket_unknown_error(scope, resolver, "getDirectory");
        return;
    };
    super::super::opfs::resolve_opfs_root(scope, resolver, locator);
}

fn storage_bucket_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = branded_promise_resolver(
        scope,
        args,
        rv,
        "StorageBucket",
        method,
        STORAGE_BUCKET_BRAND_SLOT,
    )?;
    if !storage_bucket_receiver_execution_context_is_live(scope, args.this()) {
        reject_storage_bucket_invalid_state(scope, resolver, method);
        return None;
    }
    Some(resolver)
}

fn storage_bucket_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, STORAGE_BUCKET_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn build_storage_bucket_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: StorageBucketIdentity,
) -> v8::Local<'s, v8::Object> {
    let bucket = StorageBucketObjectDeclaration::new()
        .bind(scope)
        .expect("StorageBucket declaration should bind");
    let handle = StorageBucketHandle {
        indexed_db_storage_key: identity.indexed_db_storage_key(),
        identity,
    };
    set_storage_bucket_handle_slots(scope, bucket, &handle);
    bucket
}

fn build_storage_bucket_cache_storage_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &StorageBucketHandle,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    let _ = StorageBucketCacheStorageObjectDeclaration::default().bind_into(scope, object);
    set_storage_bucket_handle_slots(scope, object, handle);
    object
}

fn build_storage_bucket_cache_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: &StorageBucketHandle,
    cache_name: &str,
    cache_id: StorageBucketCacheId,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    let _ =
        StorageBucketCacheObjectDeclaration::new(cache_name.to_owned(), cache_id.get().to_string())
            .bind_into(scope, object);
    set_storage_bucket_handle_slots(scope, object, bucket);
    if let Some(store) = current_storage_bucket_store(scope) {
        let identity = bucket.identity.clone();
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, object, move || {
            store
                .lock()
                .release_cache_handle_for_identity(&identity, cache_id);
        });
    }
    object
}

fn set_storage_bucket_handle_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    handle: &StorageBucketHandle,
) {
    let origin =
        v8_string(scope, handle.identity.storage_key()).unwrap_or_else(|| v8str(scope, ""));
    set_private_value(scope, object, STORAGE_BUCKET_ORIGIN_SLOT, origin.into());
    let storage_key =
        v8_string(scope, &handle.indexed_db_storage_key).unwrap_or_else(|| v8str(scope, ""));
    set_private_value(
        scope,
        object,
        STORAGE_BUCKET_STORAGE_KEY_SLOT,
        storage_key.into(),
    );
    let name = v8_string(scope, handle.identity.name()).unwrap_or_else(|| v8str(scope, ""));
    set_private_value(scope, object, STORAGE_BUCKET_NAME_SLOT, name.into());
    let bucket_id = v8_string(scope, &handle.identity.bucket_id().get().to_string())
        .unwrap_or_else(|| v8str(scope, "0"));
    set_private_value(scope, object, STORAGE_BUCKET_ID_SLOT, bucket_id.into());
}

fn storage_bucket_live_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
    stale_error: StorageBucketStaleError,
) -> Option<StorageBucketHandle> {
    let Some(handle) = storage_bucket_handle(scope, bucket) else {
        let message = format!("StorageBucket.{method} bucket handle is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    if !storage_bucket_handle_is_current(scope, &handle, resolver, method, stale_error) {
        return None;
    }
    Some(handle)
}

fn storage_bucket_cache_storage_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = branded_promise_resolver(
        scope,
        args,
        rv,
        "CacheStorage",
        method,
        STORAGE_BUCKET_CACHE_STORAGE_BRAND_SLOT,
    )?;
    if !storage_bucket_receiver_execution_context_is_live(scope, args.this()) {
        reject_storage_bucket_invalid_state(scope, resolver, method);
        return None;
    }
    Some(resolver)
}

fn storage_bucket_cache_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = branded_promise_resolver(
        scope,
        args,
        rv,
        "Cache",
        method,
        STORAGE_BUCKET_CACHE_BRAND_SLOT,
    )?;
    if !storage_bucket_receiver_execution_context_is_live(scope, args.this()) {
        reject_storage_bucket_invalid_state(scope, resolver, method);
        return None;
    }
    Some(resolver)
}

fn storage_bucket_receiver_execution_context_is_live<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        // Worker StorageBucket objects have no Window execution-context registry.
        return true;
    };
    let Some(relevant_context) = receiver.get_creation_context(scope) else {
        return false;
    };
    // SAFETY: the current V8 context owns a bridge ref for this host pointer.
    let host = unsafe { &*host_ptr };
    host.window_execution_context_identity_for_v8_context(scope, relevant_context)
        .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
}

fn branded_promise_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    interface: &'static str,
    method: &'static str,
    brand_slot: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    rv.set(resolver.get_promise(scope).into());
    if !get_private_value(scope, args.this(), brand_slot)
        .is_some_and(|value| value.boolean_value(scope))
    {
        reject_illegal_invocation(scope, resolver, interface, method);
        return None;
    }
    Some(resolver)
}

fn storage_bucket_cache_storage_live_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache_storage: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) -> Option<StorageBucketHandle> {
    let Some(handle) = storage_bucket_handle(scope, cache_storage) else {
        let message = format!("CacheStorage.{method} bucket handle is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    if !storage_bucket_handle_is_current(
        scope,
        &handle,
        resolver,
        method,
        StorageBucketStaleError::Unknown,
    ) {
        return None;
    }
    Some(handle)
}

fn storage_bucket_cache_live_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) -> Option<StorageBucketCacheHandle> {
    let Some(bucket) = storage_bucket_handle(scope, cache) else {
        let message = format!("Cache.{method} bucket handle is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    let Some(cache_name) = get_private_value(scope, cache, STORAGE_BUCKET_CACHE_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        let message = format!("Cache.{method} cache name is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    let Some(cache_id) = get_private_value(scope, cache, STORAGE_BUCKET_CACHE_ID_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| value.parse::<u64>().ok())
        .map(StorageBucketCacheId::from_raw)
    else {
        let message = format!("Cache.{method} cache identity is unavailable.");
        reject_type_error(scope, resolver, &message);
        return None;
    };
    if !storage_bucket_handle_is_current(
        scope,
        &bucket,
        resolver,
        method,
        StorageBucketStaleError::Unknown,
    ) {
        return None;
    }
    Some(StorageBucketCacheHandle {
        bucket,
        cache_name,
        cache_id,
    })
}

#[derive(Clone, Copy)]
enum StorageBucketStaleError {
    Unknown,
    InvalidState,
}

fn storage_bucket_handle_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &StorageBucketHandle,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
    stale_error: StorageBucketStaleError,
) -> bool {
    let expired = with_storage_bucket_store_entry(scope, |store| {
        store.delete_bucket_if_expired(
            handle.identity.storage_key(),
            handle.identity.name(),
            storage_bucket_now_ms(),
        )
    });
    match expired {
        Some(Ok(Some(cleanup))) => {
            if let Err(error) = complete_storage_bucket_deletion(scope, &cleanup) {
                reject_type_error(scope, resolver, &error.to_string());
                return false;
            }
            reject_storage_bucket_stale_error(scope, resolver, method, stale_error);
            return false;
        }
        Some(Ok(None)) => {}
        Some(Err(error)) => {
            reject_type_error(scope, resolver, &error.to_string());
            return false;
        }
        None => {
            let message = format!("StorageBucket.{method} storage bucket store is unavailable.");
            reject_type_error(scope, resolver, &message);
            return false;
        }
    }
    match with_storage_bucket_store_entry(scope, |store| {
        store.bucket_identity_is_live(&handle.identity)
    }) {
        Some(true) => true,
        Some(false) => {
            reject_storage_bucket_stale_error(scope, resolver, method, stale_error);
            false
        }
        None => {
            let message = format!("StorageBucket.{method} storage bucket store is unavailable.");
            reject_type_error(scope, resolver, &message);
            false
        }
    }
}

fn storage_bucket_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
) -> Option<StorageBucketHandle> {
    let origin = storage_bucket_origin(scope, bucket)?;
    let name = storage_bucket_name(scope, bucket)?;
    Some(StorageBucketHandle {
        identity: StorageBucketIdentity::new(&origin, &name, storage_bucket_id(scope, bucket)?),
        indexed_db_storage_key: storage_bucket_storage_key(scope, bucket)?,
    })
}

fn storage_bucket_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, bucket, STORAGE_BUCKET_ORIGIN_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn storage_bucket_storage_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, bucket, STORAGE_BUCKET_STORAGE_KEY_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn storage_bucket_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, bucket, STORAGE_BUCKET_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn storage_bucket_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bucket: v8::Local<'s, v8::Object>,
) -> Option<moli_storage_service::StorageBucketId> {
    get_private_value(scope, bucket, STORAGE_BUCKET_ID_SLOT)
        .and_then(|value| value.to_string(scope))
        .and_then(|value| value.to_rust_string_lossy(scope).parse::<u64>().ok())
        .and_then(moli_storage_service::StorageBucketId::new)
}

fn storage_bucket_indexed_db_usage_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &StorageBucketHandle,
) -> u64 {
    indexed_db_usage_bytes_for_storage_key(scope, &handle.indexed_db_storage_key)
}

fn storage_bucket_cache_storage_usage_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &StorageBucketHandle,
) -> u64 {
    with_storage_bucket_store_entry(scope, |store| {
        store.cache_usage_for_identity(&handle.identity)
    })
    .flatten()
    .unwrap_or(0)
}

fn complete_storage_bucket_deletion(
    scope: &mut v8::PinScope<'_, '_>,
    cleanup: &StorageBucketIdentity,
) -> anyhow::Result<()> {
    complete_storage_bucket_deletion_for_context(scope, cleanup).map(|_| ())
}

fn storage_bucket_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .unwrap_or(0.0)
}

fn storage_bucket_cached_response_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<StorageBucketCachedResponseMaterialization<'s>> {
    let (head, response) =
        match crate::network_host::materialize_response_object_head(scope, value, "Cache.put") {
            Ok(result) => result,
            Err(error) => {
                reject_type_error(scope, resolver, &error);
                return None;
            }
        };
    let body =
        match crate::network_host::materialize_response_object_body(scope, response, "Cache.put") {
            crate::network_host::MaterializedResponseBody::Ready(body) => body,
            crate::network_host::MaterializedResponseBody::Pending(promise) => {
                return Some(StorageBucketCachedResponseMaterialization::Pending { head, promise });
            }
            crate::network_host::MaterializedResponseBody::Failure(error) => {
                reject_type_error(scope, resolver, &error);
                return None;
            }
        };
    Some(StorageBucketCachedResponseMaterialization::Ready(
        storage_bucket_cached_response_from_head_body(head, body),
    ))
}

fn storage_bucket_cached_response_from_head_body(
    head: crate::network_host::MaterializedResponseHead,
    body: Vec<u8>,
) -> StorageBucketCachedResponse {
    StorageBucketCachedResponse {
        response_type: head.response_type,
        url: head
            .final_url
            .map(|url| url.to_string())
            .unwrap_or_default(),
        redirected: head.redirected,
        status: head.status,
        status_text: head.status_text,
        headers: head.headers,
        body,
    }
}

fn cache_request_info_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<CacheRequestInfo>, String> {
    if args.length() <= index {
        return Ok(None);
    }
    let value = args.get(index);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && crate::network_host::is_branded_request_object(scope, object)
        && let Some(url) = crate::network_host::request_slot_string(
            scope,
            object,
            crate::network_host::REQUEST_URL_SLOT,
        )
    {
        let method = crate::network_host::request_method(scope, object);
        let headers = crate::network_host::request_headers_entries(scope, object);
        return Ok(Some(CacheRequestInfo {
            url,
            method,
            headers,
        }));
    }
    let Some(input) = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return Ok(None);
    };
    crate::network_host::try_resolve_request_constructor_url(scope, &input).map(|url| {
        Some(CacheRequestInfo {
            url,
            method: "GET".to_owned(),
            headers: Vec::new(),
        })
    })
}

fn cache_query_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Option<CacheQueryOptions> {
    if args.length() <= index || args.get(index).is_null_or_undefined() {
        return Some(CacheQueryOptions::default());
    }
    let options = args.get(index).to_object(scope)?;
    let cache_name = cache_query_cache_name_option(scope, options)?;
    let ignore_method = cache_query_boolean_option(scope, options, "ignoreMethod")?;
    let ignore_search = cache_query_boolean_option(scope, options, "ignoreSearch")?;
    let ignore_vary = cache_query_boolean_option(scope, options, "ignoreVary")?;
    Some(CacheQueryOptions {
        ignore_search,
        ignore_method,
        ignore_vary,
        cache_name,
    })
}

fn cache_query_boolean_option(
    scope: &mut v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<bool> {
    options
        .get(scope, v8_string(scope, name)?.into())
        .map(|value| value.boolean_value(scope))
}

fn cache_query_cache_name_option(
    scope: &mut v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Object>,
) -> Option<Option<String>> {
    let value = options.get(scope, v8str(scope, "cacheName").into())?;
    if value.is_undefined() {
        return Some(None);
    }
    value
        .to_string(scope)
        .map(|value| Some(value.to_rust_string_lossy(scope)))
}

fn storage_bucket_cache_query(
    request: CacheRequestInfo,
    options: &CacheQueryOptions,
) -> StorageBucketCacheQuery {
    StorageBucketCacheQuery {
        request_url: request.url,
        method: request.method,
        headers: request.headers,
        ignore_search: options.ignore_search,
        ignore_method: options.ignore_method,
        ignore_vary: options.ignore_vary,
    }
}

fn build_storage_bucket_cached_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: StorageBucketCachedResponse,
) -> Option<v8::Local<'s, v8::Object>> {
    if matches!(response.response_type.as_str(), "opaque" | "opaqueredirect") {
        return crate::network_host::build_filtered_cached_response_object(
            scope,
            &response.response_type,
            &response.url,
            response.body,
        );
    }
    let body: v8::Local<'s, v8::Value> = if matches!(response.status, 204 | 205 | 304) {
        v8::null(scope).into()
    } else {
        crate::blob::array_buffer_from_bytes(scope, response.body)?.into()
    };
    let init = ObjectLiteralDeclaration::bind(scope);
    init.set_string_property(
        scope,
        "status",
        v8::Integer::new(scope, response.status as i32).into(),
    );
    let status_text = v8_string(scope, &response.status_text)?;
    init.set_string_property(scope, "statusText", status_text.into());
    let headers = headers_entries_to_init_array(scope, &response.headers);
    init.set_string_property(scope, "headers", headers.into());
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "Response").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let response_obj = constructor.new_instance(scope, &[body, init.into_value()])?;
    crate::network_host::set_response_slot_string(
        scope,
        response_obj,
        crate::network_host::RESPONSE_TYPE_SLOT,
        &response.response_type,
    );
    crate::network_host::set_response_slot_string(
        scope,
        response_obj,
        crate::network_host::RESPONSE_URL_SLOT,
        &response.url,
    );
    crate::network_host::set_response_slot_bool(
        scope,
        response_obj,
        crate::network_host::RESPONSE_REDIRECTED_SLOT,
        response.redirected,
    );
    Some(response_obj)
}

fn cache_entries_to_request_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[StorageBucketCacheMatch],
) -> Option<v8::Local<'s, v8::Array>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "Request").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let requests = v8::Array::new(scope, entries.len() as i32);
    for (index, entry) in entries.iter().enumerate() {
        let request_url = v8_string(scope, &entry.request_url)?;
        let init = ObjectLiteralDeclaration::bind(scope);
        let method = v8_string(scope, &entry.request.method)?;
        init.set_string_property(scope, "method", method.into());
        let headers = headers_entries_to_init_array(scope, &entry.request.headers);
        init.set_string_property(scope, "headers", headers.into());
        let request = constructor.new_instance(scope, &[request_url.into(), init.into_value()])?;
        if requests.set_index(scope, index as u32, request.into()) != Some(true) {
            return None;
        }
    }
    Some(requests)
}

fn headers_entries_to_init_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[(String, String)],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, entries.len() as i32);
    for (index, (name, value)) in entries.iter().enumerate() {
        let pair = v8::Array::new(scope, 2);
        if let Some(name) = v8_string(scope, name) {
            let _ = pair.set_index(scope, 0, name.into());
        }
        if let Some(value) = v8_string(scope, value) {
            let _ = pair.set_index(scope, 1, value.into());
        }
        let _ = array.set_index(scope, index as u32, pair.into());
    }
    array
}

fn cache_entry_usage_bytes(
    request: &CacheRequestInfo,
    response: &StorageBucketCachedResponse,
) -> u64 {
    (request.url.len() as u64)
        .saturating_add(request.method.len() as u64)
        .saturating_add(
            request
                .headers
                .iter()
                .map(|(name, value)| name.len().saturating_add(value.len()) as u64)
                .fold(0u64, |total, bytes| total.saturating_add(bytes)),
        )
        .saturating_add(response.body.len() as u64)
        .saturating_add(
            response
                .headers
                .iter()
                .map(|(name, value)| name.len().saturating_add(value.len()) as u64)
                .fold(0u64, |total, bytes| total.saturating_add(bytes)),
        )
        .saturating_add(1)
}

fn required_dom_string_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    _method: &'static str,
    _interface: &'static str,
) -> Option<String> {
    if args.length() <= index {
        return None;
    }
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn valid_storage_bucket_name(name: &str) -> bool {
    if name.is_empty() || name.len() >= 64 || !name.is_ascii() {
        return false;
    }
    name.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || (index != 0 && matches!(byte, b'-' | b'_'))
    })
}

fn storage_bucket_open_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<StorageBucketOpenOptions> {
    if args.length() <= 1 || args.get(1).is_null_or_undefined() {
        return Some(StorageBucketOpenOptions::default());
    }
    let Some(options) = args.get(1).to_object(scope) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open options must be an object.",
        );
        return None;
    };
    Some(StorageBucketOpenOptions {
        expires: storage_bucket_open_expires_option(scope, options, resolver)?,
        durability: storage_bucket_open_durability_option(scope, options, resolver)?,
        quota: storage_bucket_open_quota_option(scope, options, resolver)?,
        persisted: storage_bucket_open_persisted_option(scope, options, resolver)?,
    })
}

fn storage_bucket_open_expires_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<Option<f64>> {
    let Some(expires) = options.get(scope, v8str(scope, "expires").into()) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open could not read options.expires.",
        );
        return None;
    };
    if expires.is_undefined() || expires.is_null() {
        return Some(None);
    }
    let expires = expires.number_value(scope).unwrap_or(f64::NAN);
    if !expires.is_finite() {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open options.expires must be a finite timestamp.",
        );
        return None;
    }
    Some(Some(expires))
}

fn storage_bucket_open_durability_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<Option<StorageBucketDurability>> {
    let Some(durability) = options.get(scope, v8str(scope, "durability").into()) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open could not read options.durability.",
        );
        return None;
    };
    if durability.is_undefined() {
        return Some(None);
    }
    let Some(durability) = durability.to_string(scope) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open options.durability must be a string.",
        );
        return None;
    };
    match durability.to_rust_string_lossy(scope).as_str() {
        "relaxed" => Some(Some(StorageBucketDurability::Relaxed)),
        "strict" => Some(Some(StorageBucketDurability::Strict)),
        _ => {
            reject_type_error(
                scope,
                resolver,
                "StorageBucketManager.open options.durability must be 'strict' or 'relaxed'.",
            );
            None
        }
    }
}

fn storage_bucket_open_quota_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<Option<u64>> {
    let Some(quota) = options.get(scope, v8str(scope, "quota").into()) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open could not read options.quota.",
        );
        return None;
    };
    if quota.is_undefined() {
        return Some(None);
    }
    let quota = quota.number_value(scope).unwrap_or(f64::NAN);
    if !quota.is_finite() || !(1.0..=MAX_STORAGE_BUCKET_QUOTA_BYTES).contains(&quota) {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open options.quota must be between 1 and Number.MAX_SAFE_INTEGER.",
        );
        return None;
    }
    Some(Some(quota.floor() as u64))
}

fn storage_bucket_open_persisted_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<Option<bool>> {
    let Some(persisted) = options.get(scope, v8str(scope, "persisted").into()) else {
        reject_type_error(
            scope,
            resolver,
            "StorageBucketManager.open could not read options.persisted.",
        );
        return None;
    };
    if persisted.is_undefined() {
        return Some(None);
    }
    Some(Some(persisted.boolean_value(scope)))
}

fn reject_illegal_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    interface: &'static str,
    method: &'static str,
) {
    let message = format!("Failed to execute '{method}' on '{interface}': Illegal invocation.");
    reject_type_error(scope, resolver, &message);
}

fn reject_storage_bucket_unknown_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) {
    let message = format!("StorageBucket.{method} failed because the bucket no longer exists.");
    let error = crate::context_bootstrap::new_dom_exception_value(scope, &message, "UnknownError");
    let _ = resolver.reject(scope, error);
}

fn reject_storage_bucket_stale_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
    stale_error: StorageBucketStaleError,
) {
    match stale_error {
        StorageBucketStaleError::Unknown => {
            reject_storage_bucket_unknown_error(scope, resolver, method)
        }
        StorageBucketStaleError::InvalidState => {
            let message =
                format!("StorageBucket.{method} failed because the bucket no longer exists.");
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                &message,
                "InvalidStateError",
            );
            let _ = resolver.reject(scope, error);
        }
    }
}

fn reject_storage_bucket_invalid_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) {
    let message =
        format!("StorageBucket.{method} failed because its execution context has been destroyed.");
    let error =
        crate::context_bootstrap::new_dom_exception_value(scope, &message, "InvalidStateError");
    let _ = resolver.reject(scope, error);
}

fn reject_storage_bucket_security_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    interface: &'static str,
    method: &'static str,
) {
    let message = format!(
        "Failed to execute '{method}' on '{interface}': access to the Storage Buckets API is denied in this context."
    );
    let error = crate::context_bootstrap::new_dom_exception_value(scope, &message, "SecurityError");
    let _ = resolver.reject(scope, error);
}

fn reject_storage_bucket_quota_exceeded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    quota: u64,
    requested: u64,
) {
    let error = crate::context_bootstrap::new_quota_exceeded_error_value(
        scope,
        "StorageBucket quota exceeded.",
        Some(quota as f64),
        Some(requested as f64),
    );
    let _ = resolver.reject(scope, error);
}

fn reject_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn strings_to_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[String],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, value) in values.iter().enumerate() {
        let Some(value) = v8_string(scope, value) else {
            continue;
        };
        let _ = array.set_index(scope, index as u32, value.into());
    }
    array
}

pub(in crate::context_bootstrap) fn navigator_ua_data_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_ua_data_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let identity = navigator_ua_data_identity(scope, args.this());
    rv.set(build_navigator_ua_data_snapshot(scope, &identity, None).into());
}

pub(in crate::context_bootstrap) fn navigator_ua_data_get_high_entropy_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if !navigator_ua_data_receiver_branded(scope, args.this()) {
        reject_type_error(scope, resolver, "Illegal invocation");
        rv.set(resolver.get_promise(scope).into());
        return;
    }
    let requested = match navigator_ua_data_high_entropy_keys(scope, &args) {
        Ok(requested) => requested,
        Err(exception) => {
            let _ = resolver.reject(scope, exception);
            rv.set(resolver.get_promise(scope).into());
            return;
        }
    };
    let identity = navigator_ua_data_identity(scope, args.this());
    let values = build_navigator_ua_data_snapshot(scope, &identity, Some(&requested));
    let _ = resolver.resolve(scope, values.into());
    rv.set(resolver.get_promise(scope).into());
}

fn navigator_ua_data_high_entropy_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<std::collections::BTreeSet<String>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut conversion_scope = try_catch.init();
    match webidl::try_parse_args::<NavigatorUaDataGetHighEntropyValuesArgs>(
        &mut conversion_scope,
        args,
    ) {
        Ok(parsed) => Ok(parsed.hints.0.into_iter().map(|hint| hint.0).collect()),
        Err(error) => {
            webidl::throw_error(&mut conversion_scope, &error);
            let exception = conversion_scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
            conversion_scope.reset();
            Err(exception)
        }
    }
}

fn navigator_ua_data_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, NAVIGATOR_UA_DATA_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn build_navigator_ua_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &BrowserIdentityProfile,
) -> v8::Local<'s, v8::Object> {
    let brands = build_ua_brand_records(identity.brands());
    let object = NavigatorUaDataObjectDeclaration::new(
        identity.user_agent().to_owned(),
        brands,
        identity.mobile(),
        identity.platform().to_owned(),
    )
    .bind(scope)
    .expect("NavigatorUAData declaration should bind");
    set_navigator_identity_profile(scope, object, identity);
    object
}

fn build_navigator_ua_data_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &BrowserIdentityProfile,
    requested: Option<&std::collections::BTreeSet<String>>,
) -> v8::Local<'s, v8::Object> {
    let brands = build_ua_brand_records(identity.brands());
    if let Some(requested) = requested {
        let requested_key = |key: &str| requested.contains(key);
        return NavigatorUaDataHighEntropySnapshotDeclaration::new(
            requested_key("architecture").then(|| identity.architecture().to_owned()),
            requested_key("bitness").then(|| identity.bitness().to_owned()),
            brands,
            requested_key("formFactors").then(|| identity.form_factors().to_vec()),
            requested_key("fullVersionList")
                .then(|| build_ua_brand_records(identity.full_version_list())),
            identity.mobile(),
            requested_key("model").then(|| identity.model().to_owned()),
            identity.platform().to_owned(),
            requested_key("platformVersion").then(|| identity.platform_version().to_owned()),
            requested_key("uaFullVersion").then(|| identity.full_version().to_owned()),
            requested_key("wow64").then(|| identity.wow64()),
        )
        .bind(scope)
        .expect("NavigatorUAData high entropy snapshot declaration should bind");
    }
    NavigatorUaDataSnapshotDeclaration::new(
        brands,
        identity.mobile(),
        identity.platform().to_owned(),
    )
    .bind(scope)
    .expect("NavigatorUAData snapshot declaration should bind")
}

fn navigator_ua_data_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> BrowserIdentityProfile {
    if let Some(identity) = navigator_identity_profile(scope, object) {
        return identity;
    }
    get_private_value(scope, object, NAVIGATOR_UA_DATA_USER_AGENT_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|user_agent| {
            BrowserIdentityProfile::new(user_agent, moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE)
        })
        .unwrap_or_default()
}

fn build_ua_brand_records(
    brands: &[moli_browser_profile::BrowserBrandVersion],
) -> Vec<NavigatorUaBrandEntryDeclaration> {
    brands
        .iter()
        .map(|entry| {
            NavigatorUaBrandEntryDeclaration::new(entry.brand.clone(), entry.version.clone())
        })
        .collect()
}

struct StorageUsageSnapshot {
    total: u64,
    quota: u64,
    indexed_db: u64,
    cache_storage: u64,
    opfs: u64,
}

fn build_storage_estimate_object_with_quota<'s>(
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

pub(in crate::context_bootstrap) fn navigator_media_devices_enumerate_devices_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if !media_devices_receiver_branded(scope, args.this()) {
        reject_type_error(scope, resolver, "Illegal invocation");
        rv.set(resolver.get_promise(scope).into());
        return;
    }
    let devices = v8::Array::new(scope, 0);
    let _ = resolver.resolve(scope, devices.into());
    rv.set(resolver.get_promise(scope).into());
}

pub(in crate::context_bootstrap) fn navigator_media_devices_get_user_media_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if !media_devices_receiver_branded(scope, args.this()) {
        reject_type_error(scope, resolver, "Illegal invocation");
        rv.set(resolver.get_promise(scope).into());
        return;
    }
    let message = v8_string(scope, "Requested media device not found")
        .unwrap_or_else(|| v8_string(scope, "").unwrap());
    let error = v8::Exception::error(scope, message);
    let _ = resolver.reject(scope, error);
    rv.set(resolver.get_promise(scope).into());
}

fn media_devices_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, MEDIA_DEVICES_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

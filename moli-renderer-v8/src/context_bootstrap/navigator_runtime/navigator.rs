use super::super::window_runtime::{
    MEDIA_DEVICES_BRAND_SLOT, PERMISSIONS_BRAND_SLOT, build_legacy_storage_quota_object,
    build_navigator_ua_data_object, install_initial_service_worker_ready_promise,
    navigator_get_battery_callback, navigator_java_enabled_callback,
    navigator_media_devices_enumerate_devices_callback,
    navigator_media_devices_get_user_media_callback, navigator_permissions_query_callback,
    navigator_send_beacon_callback, navigator_service_worker_controller_getter_callback,
    navigator_service_worker_controllerchange_handler_getter_callback,
    navigator_service_worker_controllerchange_handler_setter_callback,
    navigator_service_worker_get_registration_callback,
    navigator_service_worker_get_registrations_callback,
    navigator_service_worker_message_handler_getter_callback,
    navigator_service_worker_message_handler_setter_callback,
    navigator_service_worker_messageerror_handler_getter_callback,
    navigator_service_worker_messageerror_handler_setter_callback,
    navigator_service_worker_register_callback, navigator_storage_estimate_callback,
    navigator_storage_get_directory_callback, navigator_storage_persist_callback,
    navigator_storage_persisted_callback, navigator_ua_data_get_high_entropy_values_callback,
    navigator_ua_data_to_json_callback, navigator_vibrate_callback,
    permission_status_name_getter_callback, permission_status_state_getter_callback,
    service_worker_object_set_owner_scope,
};
use super::super::*;
use super::collections::{
    build_navigator_plugin_collections, install_navigator_collection_template_bindings,
};
use super::geolocation::{build_geolocation_object, install_geolocation_template_bindings};
use super::media_capabilities::{
    build_media_capabilities_object, install_media_capabilities_template_bindings,
};
use super::navigator_subobjects::{NavigatorSubobject, ensure_navigator_subobject};
use crate::document_runtime::DomHandle;
use crate::native_bridge::OwnerDispatchScope;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, serialize_v8_array,
    set_private_value, throw_type_error,
};
use moli_browser_profile::{BrowserIdentityProfile, navigator_app_version};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const NAVIGATOR_RUNTIME_DATA_KEYS: &[&str] = &[
    "userAgent",
    "appCodeName",
    "appName",
    "appVersion",
    "platform",
    "language",
    "vendor",
    "vendorSub",
    "product",
    "productSub",
    "onLine",
    "hardwareConcurrency",
    "maxTouchPoints",
    "webdriver",
    "languages",
    "mimeTypes",
    "plugins",
    "pdfViewerEnabled",
    "deviceMemory",
    "doNotTrack",
    "connection",
    "userAgentData",
    "permissions",
    "storage",
    "webkitTemporaryStorage",
    "webkitPersistentStorage",
    "mediaDevices",
    "serviceWorker",
    "clipboard",
    "userActivation",
    "storageBuckets",
    "geolocation",
    "mediaCapabilities",
];
const WORKER_NAVIGATOR_INSTALLED_SLOT: &str = "__moliWorkerNavigatorInstalled";
const WORKER_NAVIGATOR_MATERIALIZING_SLOT: &str = "__moliWorkerNavigatorMaterializing";
const WORKER_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT: &str =
    "__moliWorkerNavigatorStorageApisAvailable";
const WORKER_NAVIGATOR_USER_AGENT_SEED_SLOT: &str = "__moliWorkerNavigatorUserAgentSeed";
const WORKER_NAVIGATOR_ACCEPT_LANGUAGE_SEED_SLOT: &str = "__moliWorkerNavigatorAcceptLanguageSeed";
const WORKER_NAVIGATOR_BACKING_SLOT: &str = "__moliWorkerNavigatorBacking";

const NAVIGATOR_BRAND_SLOT: &str = "__moliNavigatorBrand";
const NAVIGATOR_STORAGE_OWNER_CHILD_HANDLE_SLOT: &str = "__moliNavigatorStorageOwnerChildHandle";
const NAVIGATOR_STORAGE_OWNER_POPUP_ID_SLOT: &str = "__moliNavigatorStorageOwnerPopupId";
const NAVIGATOR_ACCEPT_LANGUAGE_SLOT: &str = "__moliNavigatorAcceptLanguage";
pub(in crate::context_bootstrap) const NAVIGATOR_IDENTITY_PROFILE_SLOT: &str =
    "__moliNavigatorIdentityProfile";
pub(in crate::context_bootstrap) const STORAGE_MANAGER_BRAND_SLOT: &str =
    "__moliStorageManagerBrand";
pub(in crate::context_bootstrap) const STORAGE_MANAGER_CHILD_HANDLE_SLOT: &str =
    "__moliStorageManagerChildHandle";
pub(in crate::context_bootstrap) const STORAGE_MANAGER_POPUP_ID_SLOT: &str =
    "__moliStorageManagerPopupId";
pub(in crate::context_bootstrap) const STORAGE_BUCKET_MANAGER_BRAND_SLOT: &str =
    "__moliStorageBucketManagerBrand";
pub(in crate::context_bootstrap) const STORAGE_BUCKET_MANAGER_CHILD_HANDLE_SLOT: &str =
    "__moliStorageBucketManagerChildHandle";
pub(in crate::context_bootstrap) const STORAGE_BUCKET_MANAGER_POPUP_ID_SLOT: &str =
    "__moliStorageBucketManagerPopupId";
const SERVICE_WORKER_CONTAINER_LISTENERS_SLOT: &str = "__moliServiceWorkerContainerEvents";
const SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT: &str = "__moliServiceWorkerContainerOnmessage";
const SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT: &str =
    "__moliServiceWorkerContainerOnmessageerror";
const SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT: &str =
    "__moliServiceWorkerContainerOncontrollerchange";
const SERVICE_WORKER_CONTAINER_CONTROLLER_SLOT: &str = "__moliServiceWorkerContainerController";
const CLIPBOARD_TEXT_SLOT: &str = "__moliClipboardText";
pub(in crate::context_bootstrap) const SERVICE_WORKER_OWNER_TOKEN_SLOT: &str =
    "__moliServiceWorkerOwner";
const USER_ACTIVATION_BRAND_SLOT: &str = "__moliUserActivationBrand";
const NAVIGATOR_CONNECTION_BRAND_SLOT: &str = "__moliNavigatorConnectionBrand";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Permissions", allow_empty)]
struct PermissionsObjectDeclaration {
    #[webapi(slot = PERMISSIONS_BRAND_SLOT, init = true)]
    brand: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Permissions", enumerable)]
struct PermissionsPrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = navigator_permissions_query_callback)]
    query: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Navigator")]
struct NavigatorObjectDeclaration<'scope> {
    #[webapi(slot = NAVIGATOR_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = NAVIGATOR_RUNTIME_DATA_SLOT)]
    runtime_data: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Navigator", enumerable)]
struct NavigatorRuntimeDataPrototypeDeclaration {
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 0))]
    user_agent: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 1))]
    app_code_name: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 2))]
    app_name: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 3))]
    app_version: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 4))]
    platform: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 5))]
    language: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 6))]
    vendor: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 7))]
    vendor_sub: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 8))]
    product: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 9))]
    product_sub: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 10))]
    on_line: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 11))]
    hardware_concurrency: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 12))]
    max_touch_points: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 13))]
    webdriver: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 14))]
    languages: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 15))]
    mime_types: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 16))]
    plugins: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 17))]
    pdf_viewer_enabled: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 18))]
    device_memory: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 19))]
    do_not_track: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 20))]
    connection: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 21))]
    user_agent_data: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 22))]
    permissions: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 23))]
    storage: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 24))]
    webkit_temporary_storage: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 25))]
    webkit_persistent_storage: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 26))]
    media_devices: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 27))]
    service_worker: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 28))]
    clipboard: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 29))]
    user_activation: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 30))]
    storage_buckets: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 31))]
    geolocation: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 32))]
    media_capabilities: (),
    #[webapi(accessor_property, getter = navigator_cookie_enabled_getter_callback)]
    cookie_enabled: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Navigator")]
struct NavigatorPrototypeMethodsDeclaration {
    #[webapi(method, enumerable, length = 0, callback = navigator_java_enabled_callback)]
    java_enabled: (),

    #[webapi(method, enumerable, length = 1, callback = navigator_send_beacon_callback)]
    send_beacon: (),

    #[webapi(method, enumerable, length = 0, callback = navigator_get_battery_callback)]
    get_battery: (),

    #[webapi(method, enumerable, length = 1, callback = navigator_vibrate_callback)]
    vibrate: (),

    #[webapi(method = "getAutoplayPolicy", enumerable, length = 1, callback = navigator_get_autoplay_policy_callback)]
    get_autoplay_policy: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "WorkerNavigator")]
struct WorkerNavigatorObjectDeclaration<'scope> {
    #[webapi(slot = NAVIGATOR_RUNTIME_DATA_SLOT)]
    runtime_data: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "WorkerNavigator", enumerable)]
struct WorkerNavigatorRuntimeDataPrototypeDeclaration {
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 0))]
    user_agent: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 1))]
    app_code_name: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 2))]
    app_name: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 3))]
    app_version: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 4))]
    platform: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 5))]
    language: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 6))]
    vendor: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 7))]
    vendor_sub: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 8))]
    product: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 9))]
    product_sub: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 10))]
    on_line: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 11))]
    hardware_concurrency: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 14))]
    languages: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 18))]
    device_memory: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 21))]
    user_agent_data: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 23))]
    storage: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 27))]
    service_worker: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 30))]
    storage_buckets: (),
    #[webapi(accessor_property, getter = navigator_runtime_data_getter_callback, data = callback_data_index_value(scope, 32))]
    media_capabilities: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "PermissionStatus", enumerable)]
struct PermissionStatusPrototypeDeclaration {
    #[webapi(accessor_property, getter = permission_status_name_getter_callback)]
    name: (),
    #[webapi(accessor_property, getter = permission_status_state_getter_callback)]
    state: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct NavigatorConnectionDeclaration {
    #[webapi(slot = NAVIGATOR_CONNECTION_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(data_property, name = "type", enumerable, value = DEFAULT_CONNECTION_TYPE)]
    connection_type: (),

    #[webapi(data_property, enumerable, value = DEFAULT_CONNECTION_DOWNLINK_MAX)]
    downlink_max: (),

    #[webapi(data_property, enumerable, value = DEFAULT_CONNECTION_EFFECTIVE_TYPE)]
    effective_type: (),

    #[webapi(data_property, enumerable, value = DEFAULT_CONNECTION_DOWNLINK)]
    downlink: (),

    #[webapi(data_property, enumerable, value = DEFAULT_CONNECTION_RTT)]
    rtt: (),

    #[webapi(data_property, enumerable, value = DEFAULT_CONNECTION_SAVE_DATA)]
    save_data: (),

    #[webapi(data_property, enumerable, init = "null")]
    onchange: (),

    #[webapi(
        method,
        enumerable,
        length = 2,
        callback = navigator_connection_event_target_noop_callback
    )]
    add_event_listener: (),

    #[webapi(
        method,
        enumerable,
        length = 2,
        callback = navigator_connection_event_target_noop_callback
    )]
    remove_event_listener: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "StorageManager")]
struct StorageManagerObjectDeclaration {
    #[webapi(slot, name = STORAGE_MANAGER_BRAND_SLOT, constructor_default = true)]
    brand: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "StorageManager", enumerable)]
struct StorageManagerTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = navigator_storage_persisted_callback)]
    persisted: (),

    #[webapi(method, length = 0, callback = navigator_storage_persist_callback)]
    persist: (),

    #[webapi(method, length = 0, callback = navigator_storage_estimate_callback)]
    estimate: (),

    #[webapi(method, length = 0, callback = navigator_storage_get_directory_callback)]
    get_directory: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "StorageManager", enumerable)]
struct StorageManagerWorkerTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = navigator_storage_persisted_callback)]
    persisted: (),

    #[webapi(method, length = 0, callback = navigator_storage_estimate_callback)]
    estimate: (),

    #[webapi(method, length = 0, callback = navigator_storage_get_directory_callback)]
    get_directory: (),
}

pub(in crate::context_bootstrap) fn build_storage_manager_worker_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    StorageManagerWorkerTemplateMethodsDeclaration::build(scope)
}

pub(in crate::context_bootstrap) fn install_storage_manager_constructor_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    include_persist: bool,
) {
    let prototype = template.prototype_template(scope);
    if include_persist {
        StorageManagerTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    } else {
        StorageManagerWorkerTemplateMethodsDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
}

#[derive(WebApiObject)]
#[webapi(interface = "StorageBucketManager")]
struct StorageBucketManagerObjectDeclaration {
    #[webapi(
        slot,
        name = STORAGE_BUCKET_MANAGER_BRAND_SLOT,
        constructor_default = true
    )]
    brand: bool,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "NavigatorUAData", enumerable)]
struct NavigatorUaDataPrototypeMethodsDeclaration {
    #[webapi(method, name = "toJSON", length = 0, callback = navigator_ua_data_to_json_callback)]
    to_json: (),

    #[webapi(method, length = 1, callback = navigator_ua_data_get_high_entropy_values_callback)]
    get_high_entropy_values: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "MediaDevices")]
struct MediaDevicesObjectDeclaration {
    #[webapi(slot = MEDIA_DEVICES_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(method, enumerable, length = 0, callback = navigator_media_devices_enumerate_devices_callback)]
    enumerate_devices: (),

    #[webapi(method, enumerable, length = 1, callback = navigator_media_devices_get_user_media_callback)]
    get_user_media: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerContainerDeclaration {
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = SERVICE_WORKER_CONTAINER_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT, init = "null")]
    _onmessage: (),

    #[webapi(slot = SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT, init = "null")]
    _onmessageerror: (),

    #[webapi(slot = SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT, init = "null")]
    _oncontrollerchange: (),

    #[webapi(slot = SERVICE_WORKER_CONTAINER_CONTROLLER_SLOT, init = "null")]
    _controller: (),

    #[webapi(accessor_property = "controller", enumerable, getter = navigator_service_worker_controller_getter_callback)]
    controller: (),

    #[webapi(method, enumerable, callback = navigator_service_worker_register_callback, length = 1)]
    register: (),

    #[webapi(method, enumerable, callback = navigator_service_worker_get_registration_callback, length = 1)]
    get_registration: (),

    #[webapi(method, enumerable, callback = navigator_service_worker_get_registrations_callback, length = 0)]
    get_registrations: (),

    #[webapi(method, enumerable, callback = simple_event_target_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(method, enumerable, callback = simple_event_target_remove_event_listener_callback)]
    remove_event_listener: (),

    #[webapi(method, enumerable, callback = simple_event_target_dispatch_event_callback)]
    dispatch_event: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = navigator_service_worker_message_handler_getter_callback,
        setter = navigator_service_worker_message_handler_setter_callback
    )]
    onmessage: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = navigator_service_worker_messageerror_handler_getter_callback,
        setter = navigator_service_worker_messageerror_handler_setter_callback
    )]
    onmessageerror: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = navigator_service_worker_controllerchange_handler_getter_callback,
        setter = navigator_service_worker_controllerchange_handler_setter_callback
    )]
    oncontrollerchange: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ClipboardObjectDeclaration {
    #[webapi(slot = CLIPBOARD_TEXT_SLOT, init = "")]
    text: (),

    #[webapi(method, enumerable, length = 0, callback = clipboard_read_text_callback)]
    read_text: (),

    #[webapi(method, enumerable, length = 1, callback = clipboard_write_text_callback)]
    write_text: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "UserActivation")]
struct UserActivationObjectDeclaration {
    #[webapi(slot = USER_ACTIVATION_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(accessor_property, getter = navigator_user_activation_state_getter_callback, enumerable)]
    is_active: (),

    #[webapi(accessor_property, getter = navigator_user_activation_state_getter_callback, enumerable)]
    has_been_active: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct WindowNavigatorBackingDeclaration<'scope, 'profile> {
    #[webapi(data_property, enumerable)]
    user_agent: &'profile str,

    #[webapi(data_property, enumerable)]
    app_code_name: &'static str,

    #[webapi(data_property, enumerable)]
    app_name: &'static str,

    #[webapi(data_property, enumerable)]
    app_version: &'profile str,

    #[webapi(data_property, enumerable)]
    platform: &'profile str,

    #[webapi(data_property, enumerable)]
    language: &'profile str,

    #[webapi(data_property, enumerable)]
    vendor: &'static str,

    #[webapi(data_property, enumerable)]
    vendor_sub: &'static str,

    #[webapi(data_property, enumerable)]
    product: &'static str,

    #[webapi(data_property, enumerable)]
    product_sub: &'static str,

    #[webapi(data_property, enumerable)]
    on_line: bool,

    #[webapi(data_property, enumerable)]
    hardware_concurrency: f64,

    #[webapi(data_property, enumerable)]
    max_touch_points: f64,

    #[webapi(data_property, enumerable)]
    webdriver: bool,

    #[webapi(data_property, enumerable)]
    languages: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    mime_types: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    plugins: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    pdf_viewer_enabled: bool,

    #[webapi(data_property, enumerable)]
    device_memory: f64,

    #[webapi(data_property, enumerable)]
    do_not_track: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    connection: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    user_agent_data: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    permissions: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    storage: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    webkit_temporary_storage: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    webkit_persistent_storage: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    media_devices: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    service_worker: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    clipboard: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    user_activation: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    storage_buckets: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    geolocation: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    media_capabilities: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct WorkerNavigatorBackingDeclaration<'scope, 'profile> {
    #[webapi(data_property, enumerable)]
    user_agent: &'profile str,

    #[webapi(data_property, enumerable)]
    app_code_name: &'static str,

    #[webapi(data_property, enumerable)]
    app_name: &'static str,

    #[webapi(data_property, enumerable)]
    app_version: &'profile str,

    #[webapi(data_property, enumerable)]
    platform: &'profile str,

    #[webapi(data_property, enumerable)]
    language: &'profile str,

    #[webapi(data_property, enumerable)]
    vendor: &'static str,

    #[webapi(data_property, enumerable)]
    vendor_sub: &'static str,

    #[webapi(data_property, enumerable)]
    product: &'static str,

    #[webapi(data_property, enumerable)]
    product_sub: &'static str,

    #[webapi(data_property, enumerable)]
    on_line: bool,

    #[webapi(data_property, enumerable)]
    hardware_concurrency: f64,

    #[webapi(data_property, enumerable)]
    device_memory: f64,

    #[webapi(data_property, enumerable)]
    languages: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    user_agent_data: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    storage: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    storage_buckets: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    service_worker: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    media_capabilities: v8::Local<'scope, v8::Value>,
}

fn navigator_runtime_data_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(backing) = navigator_runtime_data_object(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        NAVIGATOR_RUNTIME_DATA_KEYS,
        "Navigator runtime data keys",
    ) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    if let Some(subobject) = NavigatorSubobject::from_key(key) {
        match ensure_navigator_subobject(scope, backing, subobject) {
            Ok(value) => rv.set(value),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    match backing.get(scope, v8str(scope, key).into()) {
        Some(value) => rv.set(value),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn navigator_cookie_enabled_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let enabled = context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { (&*host_ptr).browser_cookie_enabled() });
    rv.set(v8::Boolean::new(scope, enabled).into());
}

pub(in crate::context_bootstrap) fn navigator_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigator: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, navigator, NAVIGATOR_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn current_protocol_user_gesture_activation(scope: &mut v8::PinScope<'_, '_>) -> bool {
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { (&*host_ptr).protocol_user_gesture_activation() })
}

fn navigator_user_activation_state_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if get_private_value(scope, args.this(), USER_ACTIVATION_BRAND_SLOT)
        .is_none_or(|value| !value.boolean_value(scope))
    {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let active = current_protocol_user_gesture_activation(scope);
    rv.set(v8::Boolean::new(scope, active).into());
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

fn clipboard_read_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CLIPBOARD_TEXT_SLOT)
        .filter(|value| value.is_string())
        .unwrap_or_else(|| v8::String::empty(scope).into());
    set_resolved_promise(scope, &mut rv, value);
}

fn clipboard_write_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let text = args
        .get(0)
        .to_string(scope)
        .unwrap_or_else(|| v8::String::empty(scope));
    set_private_value(scope, args.this(), CLIPBOARD_TEXT_SLOT, text.into());
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn navigator_connection_event_target_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn navigator_get_autoplay_policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'getAutoplayPolicy': 1 argument required",
        );
        return;
    }

    let argument = args.get(0);
    let supported = if let Ok(object) = v8::Local::<v8::Object>::try_from(argument) {
        let is_media_element =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)
                .ok()
                .is_some_and(|(runtime_ptr, handle)| {
                    unsafe { &*runtime_ptr }
                        .dom_host()
                        .node(handle)
                        .and_then(crate::dom::native::Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("audio") || element.is_html_element("video")
                        })
                });
        is_media_element || super::super::web_audio_runtime::is_audio_context_object(scope, object)
    } else {
        argument.to_string(scope).is_some_and(|value| {
            matches!(
                value.to_rust_string_lossy(scope).as_str(),
                "mediaelement" | "audiocontext"
            )
        })
    };

    if !supported {
        throw_type_error(
            scope,
            "Failed to execute 'getAutoplayPolicy': unsupported argument",
        );
        return;
    }
    if let Some(policy) = v8_string(scope, "allowed") {
        rv.set(policy.into());
    }
}

fn navigator_connection_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, NAVIGATOR_CONNECTION_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn install_navigator_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    install_geolocation_template_bindings(scope, template, interface_name);
    install_navigator_collection_template_bindings(scope, template, interface_name);
    install_media_capabilities_template_bindings(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Navigator" => {
            NavigatorRuntimeDataPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            NavigatorPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "WorkerNavigator" => {
            WorkerNavigatorRuntimeDataPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "Permissions" => {
            PermissionsPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "PermissionStatus" => {
            PermissionStatusPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "NavigatorUAData" => {
            NavigatorUaDataPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn filter_navigator_secure_context_exposure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    secure_context: bool,
) -> Result<()> {
    if !secure_context {
        delete_object_property(scope, prototype, "storage")?;
        delete_object_property(scope, prototype, "storageBuckets")?;
        delete_object_property(scope, prototype, "serviceWorker")?;
        delete_object_property(scope, prototype, "userAgentData")?;
    }
    Ok(())
}

fn filter_worker_navigator_secure_context_exposure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    secure_context: bool,
) -> Result<()> {
    if !secure_context {
        delete_object_property(scope, prototype, "storage")?;
        delete_object_property(scope, prototype, "storageBuckets")?;
        delete_object_property(scope, prototype, "serviceWorker")?;
        delete_object_property(scope, prototype, "userAgentData")?;
    }
    Ok(())
}

fn delete_object_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> Result<()> {
    object
        .delete(scope, v8str(scope, name).into())
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to remove unavailable Navigator.{name} property"))
}

fn build_languages_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    languages: &[String],
) -> v8::Local<'s, v8::Array> {
    serialize_v8_array(scope, languages)
        .or_else(|| serialize_v8_array(scope, [""]))
        .unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn build_navigator_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    NavigatorConnectionDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind navigator.connection object: {error}"))
}

fn build_service_worker_container<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) -> Result<v8::Local<'s, v8::Object>> {
    let service_worker = ServiceWorkerContainerDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind navigator.serviceWorker object: {error}"))?;
    let owner = owner_child
        .map(OwnerDispatchScope::Child)
        .or_else(|| owner_popup.map(OwnerDispatchScope::LightweightPopup))
        .unwrap_or(OwnerDispatchScope::Top);
    service_worker_object_set_owner_scope(scope, service_worker, owner);
    install_initial_service_worker_ready_promise(scope, service_worker);
    Ok(service_worker)
}

pub(in crate::context_bootstrap) fn service_worker_owner_token_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: OwnerDispatchScope,
) -> v8::Local<'s, v8::Value> {
    let token = match owner {
        OwnerDispatchScope::Top => None,
        OwnerDispatchScope::Child(handle) => Some(format!("child:{}", handle.index())),
        OwnerDispatchScope::LightweightPopup(popup_id) => Some(format!("popup:{popup_id}")),
    };
    token
        .and_then(|token| v8_string(scope, &token))
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn build_user_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    UserActivationObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind UserActivation object: {error}"))
}

fn build_navigator_plugin_collection_subobject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    requested: NavigatorSubobject,
) -> Result<v8::Local<'s, v8::Value>> {
    let collections = build_navigator_plugin_collections(scope)
        .ok_or_else(|| anyhow!("failed to build navigator plugin collections"))?;
    let mime_types: v8::Local<'s, v8::Value> = collections.mime_types.into();
    let plugins: v8::Local<'s, v8::Value> = collections.plugins.into();
    if backing.set(scope, v8str(scope, "mimeTypes").into(), mime_types) != Some(true)
        || backing.set(scope, v8str(scope, "plugins").into(), plugins) != Some(true)
    {
        return Err(anyhow!("failed to cache navigator plugin collections"));
    }
    match requested {
        NavigatorSubobject::MimeTypes => Ok(mime_types),
        NavigatorSubobject::Plugins => Ok(plugins),
        _ => Err(anyhow!(
            "navigator plugin collection builder received an unrelated subobject"
        )),
    }
}

pub(super) fn build_lazy_navigator_subobject_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    subobject: NavigatorSubobject,
) -> Result<v8::Local<'s, v8::Value>> {
    let (owner_child, owner_popup) = navigator_backing_owner(scope, backing);
    let value: v8::Local<'s, v8::Value> = match subobject {
        NavigatorSubobject::Languages => {
            let languages = effective_navigator_languages(scope, backing);
            build_languages_array(scope, &languages).into()
        }
        NavigatorSubobject::MimeTypes | NavigatorSubobject::Plugins => {
            build_navigator_plugin_collection_subobject(scope, backing, subobject)?
        }
        NavigatorSubobject::Connection => build_navigator_connection(scope)?.into(),
        NavigatorSubobject::UserAgentData => {
            let identity = navigator_identity_from_backing(scope, backing);
            build_navigator_ua_data_object(scope, &identity).into()
        }
        NavigatorSubobject::Permissions => PermissionsObjectDeclaration::default()
            .bind(scope)
            .map_err(|error| anyhow!("failed to bind Permissions object: {error}"))?
            .into(),
        NavigatorSubobject::Storage => {
            build_storage_manager(scope, owner_child, owner_popup)?.into()
        }
        NavigatorSubobject::WebkitTemporaryStorage
        | NavigatorSubobject::WebkitPersistentStorage => {
            build_legacy_storage_quota_object(scope)?.into()
        }
        NavigatorSubobject::MediaDevices => MediaDevicesObjectDeclaration::default()
            .bind(scope)
            .map_err(|error| anyhow!("failed to bind MediaDevices object: {error}"))?
            .into(),
        NavigatorSubobject::ServiceWorker => {
            build_service_worker_container(scope, owner_child, owner_popup)?.into()
        }
        NavigatorSubobject::Clipboard => ClipboardObjectDeclaration::default()
            .bind(scope)
            .map_err(|error| anyhow!("failed to bind navigator.clipboard object: {error}"))?
            .into(),
        NavigatorSubobject::UserActivation => build_user_activation(scope)?.into(),
        NavigatorSubobject::StorageBuckets => {
            build_storage_bucket_manager(scope, owner_child, owner_popup)?.into()
        }
        NavigatorSubobject::Geolocation => {
            let secure_context =
                navigator_geolocation_secure_context_available(scope, owner_child, owner_popup);
            build_geolocation_object(scope, secure_context, owner_child)?.into()
        }
        NavigatorSubobject::MediaCapabilities => {
            let worker = get_private_value(scope, backing, WORKER_NAVIGATOR_BACKING_SLOT)
                .is_some_and(|value| value.boolean_value(scope));
            let secure_context = worker
                || navigator_geolocation_secure_context_available(scope, owner_child, owner_popup);
            build_media_capabilities_object(scope, secure_context, worker)?.into()
        }
    };
    Ok(value)
}

fn navigator_backing_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
) -> (Option<DomHandle>, Option<u64>) {
    let owner_child = get_private_value(scope, backing, NAVIGATOR_STORAGE_OWNER_CHILD_HANDLE_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize));
    let owner_popup = get_private_value(scope, backing, NAVIGATOR_STORAGE_OWNER_POPUP_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (popup_id, lossless) = value.u64_value();
            (lossless && popup_id != 0).then_some(popup_id)
        });
    (owner_child, owner_popup)
}

fn navigator_geolocation_secure_context_available(
    scope: &mut v8::PinScope<'_, '_>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    let url = if let Some(owner_child) = owner_child {
        host.child_browsing_context_secure_context_url(owner_child)
    } else if let Some(owner_popup) = owner_popup {
        host.lightweight_popup_document_url(owner_popup)
            .or_else(|| Some(host.document_url().clone()))
    } else {
        Some(host.document_url().clone())
    };
    url.is_some_and(|url| moli_url::is_potentially_trustworthy_url(&url))
}

fn effective_navigator_languages<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    navigator_identity_from_backing(scope, backing)
        .languages()
        .to_vec()
}

fn navigator_identity_from_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
) -> BrowserIdentityProfile {
    if let Some(identity) = navigator_identity_profile(scope, backing) {
        return identity;
    }
    let user_agent = backing
        .get(scope, v8str(scope, "userAgent").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| moli_browser_profile::DEFAULT_USER_AGENT.to_owned());
    let accept_language = get_private_value(scope, backing, NAVIGATOR_ACCEPT_LANGUAGE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE.to_owned());
    BrowserIdentityProfile::new(user_agent, accept_language)
}

pub(in crate::context_bootstrap) fn set_navigator_identity_profile<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    identity: &BrowserIdentityProfile,
) {
    let Ok(serialized) = serde_json::to_string(identity) else {
        return;
    };
    let Some(value) = v8_string(scope, &serialized) else {
        return;
    };
    set_private_value(scope, object, NAVIGATOR_IDENTITY_PROFILE_SLOT, value.into());
}

pub(in crate::context_bootstrap) fn navigator_identity_profile<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BrowserIdentityProfile> {
    let serialized = get_private_value(scope, object, NAVIGATOR_IDENTITY_PROFILE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&serialized).ok()
}

fn build_window_navigator_backing_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
    identity_override: Option<&BrowserIdentityProfile>,
) -> Result<v8::Local<'s, v8::Object>> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    let identity = identity_override.cloned().unwrap_or_default();
    let user_agent = identity.user_agent();
    let do_not_track = v8::null(scope).into();

    WindowNavigatorBackingDeclaration {
        user_agent,
        app_code_name: DEFAULT_NAVIGATOR_APP_CODE_NAME,
        app_name: DEFAULT_NAVIGATOR_APP_NAME,
        app_version: navigator_app_version(user_agent),
        platform: identity.navigator_platform(),
        language: identity.language(),
        vendor: DEFAULT_NAVIGATOR_VENDOR,
        vendor_sub: DEFAULT_NAVIGATOR_VENDOR_SUB,
        product: DEFAULT_NAVIGATOR_PRODUCT,
        product_sub: DEFAULT_NAVIGATOR_PRODUCT_SUB,
        on_line: DEFAULT_NAVIGATOR_ONLINE,
        hardware_concurrency: profile.hardware_concurrency,
        max_touch_points: profile.max_touch_points,
        webdriver: DEFAULT_NAVIGATOR_WEBDRIVER,
        languages: v8::undefined(scope).into(),
        mime_types: v8::undefined(scope).into(),
        plugins: v8::undefined(scope).into(),
        pdf_viewer_enabled: DEFAULT_NAVIGATOR_PDF_VIEWER_ENABLED,
        device_memory: DEFAULT_NAVIGATOR_DEVICE_MEMORY,
        do_not_track,
        connection: v8::undefined(scope).into(),
        user_agent_data: v8::undefined(scope).into(),
        permissions: v8::undefined(scope).into(),
        storage: v8::undefined(scope).into(),
        webkit_temporary_storage: v8::undefined(scope).into(),
        webkit_persistent_storage: v8::undefined(scope).into(),
        media_devices: v8::undefined(scope).into(),
        service_worker: v8::undefined(scope).into(),
        clipboard: v8::undefined(scope).into(),
        user_activation: v8::undefined(scope).into(),
        storage_buckets: v8::undefined(scope).into(),
        geolocation: v8::undefined(scope).into(),
        media_capabilities: v8::undefined(scope).into(),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to bind Navigator backing object: {error}"))
    .inspect(|&backing| {
        set_navigator_identity_profile(scope, backing, &identity);
        set_navigator_storage_owner(scope, backing, owner_child, owner_popup);
        if let Some(accept_language) = v8_string(scope, identity.accept_language()) {
            set_private_value(
                scope,
                backing,
                NAVIGATOR_ACCEPT_LANGUAGE_SLOT,
                accept_language.into(),
            );
        }
    })
}

pub(super) fn build_window_navigator_object_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_child: Option<DomHandle>,
    identity: Option<&BrowserIdentityProfile>,
    storage_apis_available: bool,
) -> Result<v8::Local<'s, v8::Object>> {
    let backing = build_window_navigator_backing_for_owner(scope, owner_child, None, identity)?;
    let navigator = NavigatorObjectDeclaration::new(backing)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind Navigator object: {error}"))?;
    if let Some(prototype) = global_constructor_prototype(scope, "Navigator") {
        filter_navigator_secure_context_exposure(scope, prototype, storage_apis_available)?;
    }
    Ok(navigator)
}

pub(super) fn update_window_navigator_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigator: v8::Local<'s, v8::Object>,
    identity: &BrowserIdentityProfile,
) -> Result<()> {
    let Some(backing) = navigator_runtime_data_object(scope, navigator) else {
        return Ok(());
    };
    let user_agent_value = v8_string(scope, identity.user_agent())
        .ok_or_else(|| anyhow!("failed to allocate navigator.userAgent string"))?;
    let _ = backing.set(
        scope,
        v8str(scope, "userAgent").into(),
        user_agent_value.into(),
    );
    let Some(app_version) = v8_string(scope, navigator_app_version(identity.user_agent())) else {
        return Err(anyhow!("failed to allocate navigator.appVersion string"));
    };
    let _ = backing.set(scope, v8str(scope, "appVersion").into(), app_version.into());
    let platform = v8_string(scope, identity.navigator_platform())
        .ok_or_else(|| anyhow!("failed to allocate navigator.platform string"))?;
    let _ = backing.set(scope, v8str(scope, "platform").into(), platform.into());
    let language = v8_string(scope, identity.language())
        .ok_or_else(|| anyhow!("failed to allocate navigator.language string"))?;
    let _ = backing.set(scope, v8str(scope, "language").into(), language.into());
    let accept_language = v8_string(scope, identity.accept_language())
        .ok_or_else(|| anyhow!("failed to allocate navigator Accept-Language seed"))?;
    set_private_value(
        scope,
        backing,
        NAVIGATOR_ACCEPT_LANGUAGE_SLOT,
        accept_language.into(),
    );
    let _ = backing.set(
        scope,
        v8str(scope, "languages").into(),
        v8::undefined(scope).into(),
    );
    let _ = backing.set(
        scope,
        v8str(scope, "userAgentData").into(),
        v8::undefined(scope).into(),
    );
    set_navigator_identity_profile(scope, backing, identity);
    Ok(())
}

pub(crate) fn build_lightweight_popup_window_navigator_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_popup: u64,
) -> Result<v8::Local<'s, v8::Object>> {
    let backing = build_window_navigator_backing_for_owner(scope, None, Some(owner_popup), None)?;
    NavigatorObjectDeclaration::new(backing)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind lightweight popup Navigator object: {error}"))
}

fn build_worker_navigator_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &BrowserIdentityProfile,
) -> Result<v8::Local<'s, v8::Object>> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    let user_agent = identity.user_agent();
    let backing = WorkerNavigatorBackingDeclaration {
        user_agent,
        app_code_name: DEFAULT_NAVIGATOR_APP_CODE_NAME,
        app_name: DEFAULT_NAVIGATOR_APP_NAME,
        app_version: navigator_app_version(user_agent),
        platform: identity.navigator_platform(),
        language: identity.language(),
        vendor: DEFAULT_NAVIGATOR_VENDOR,
        vendor_sub: DEFAULT_NAVIGATOR_VENDOR_SUB,
        product: DEFAULT_NAVIGATOR_PRODUCT,
        product_sub: DEFAULT_NAVIGATOR_PRODUCT_SUB,
        on_line: DEFAULT_NAVIGATOR_ONLINE,
        hardware_concurrency: profile.hardware_concurrency,
        device_memory: DEFAULT_NAVIGATOR_DEVICE_MEMORY,
        languages: v8::undefined(scope).into(),
        user_agent_data: v8::undefined(scope).into(),
        storage: v8::undefined(scope).into(),
        storage_buckets: v8::undefined(scope).into(),
        service_worker: v8::undefined(scope).into(),
        media_capabilities: v8::undefined(scope).into(),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to bind WorkerNavigator backing object: {error}"))?;
    set_private_value(
        scope,
        backing,
        WORKER_NAVIGATOR_BACKING_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let accept_language = v8_string(scope, identity.accept_language())
        .ok_or_else(|| anyhow!("failed to allocate WorkerNavigator Accept-Language seed"))?;
    set_private_value(
        scope,
        backing,
        NAVIGATOR_ACCEPT_LANGUAGE_SLOT,
        accept_language.into(),
    );
    set_navigator_identity_profile(scope, backing, identity);
    Ok(backing)
}

fn set_navigator_storage_owner(
    scope: &mut v8::PinScope<'_, '_>,
    backing: v8::Local<'_, v8::Object>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) {
    if let Some(owner_child) = owner_child {
        let value = v8::Number::new(scope, owner_child.index() as f64);
        set_private_value(
            scope,
            backing,
            NAVIGATOR_STORAGE_OWNER_CHILD_HANDLE_SLOT,
            value.into(),
        );
    }
    if let Some(owner_popup) = owner_popup {
        let value = v8::BigInt::new_from_u64(scope, owner_popup);
        set_private_value(
            scope,
            backing,
            NAVIGATOR_STORAGE_OWNER_POPUP_ID_SLOT,
            value.into(),
        );
    }
}

fn build_storage_bucket_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) -> Result<v8::Local<'s, v8::Object>> {
    let manager = StorageBucketManagerObjectDeclaration::new()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind StorageBucketManager object: {error}"))?;
    let child_value: v8::Local<'s, v8::Value> = owner_child
        .map(|handle| v8::Number::new(scope, handle.index() as f64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        manager,
        STORAGE_BUCKET_MANAGER_CHILD_HANDLE_SLOT,
        child_value,
    );
    let popup_value: v8::Local<'s, v8::Value> = owner_popup
        .map(|popup_id| v8::BigInt::new_from_u64(scope, popup_id).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        manager,
        STORAGE_BUCKET_MANAGER_POPUP_ID_SLOT,
        popup_value,
    );
    Ok(manager)
}

fn build_storage_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) -> Result<v8::Local<'s, v8::Object>> {
    let manager = StorageManagerObjectDeclaration::new()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind StorageManager object: {error}"))?;
    let child_value: v8::Local<'s, v8::Value> = owner_child
        .map(|handle| v8::Number::new(scope, handle.index() as f64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        manager,
        STORAGE_MANAGER_CHILD_HANDLE_SLOT,
        child_value,
    );
    let popup_value: v8::Local<'s, v8::Value> = owner_popup
        .map(|popup_id| v8::BigInt::new_from_u64(scope, popup_id).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, manager, STORAGE_MANAGER_POPUP_ID_SLOT, popup_value);
    Ok(manager)
}

pub(crate) fn install_worker_navigator_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    storage_apis_available: bool,
    identity: &BrowserIdentityProfile,
) -> Result<()> {
    let user_agent = v8_string(scope, identity.user_agent())
        .ok_or_else(|| anyhow!("failed to allocate WorkerNavigator user-agent seed"))?;
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_USER_AGENT_SEED_SLOT,
        user_agent.into(),
    );
    let accept_language = v8_string(scope, identity.accept_language())
        .ok_or_else(|| anyhow!("failed to allocate WorkerNavigator Accept-Language seed"))?;
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_ACCEPT_LANGUAGE_SEED_SLOT,
        accept_language.into(),
    );
    set_navigator_identity_profile(scope, global, identity);
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT,
        v8::Boolean::new(scope, storage_apis_available).into(),
    );
    if get_private_value(scope, global, WORKER_NAVIGATOR_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    global
        .set_lazy_data_property_with_configuration(
            scope,
            v8str(scope, "navigator").into(),
            v8::LazyDataPropertyConfiguration::new(worker_navigator_lazy_getter)
                .property_attribute(v8::PropertyAttribute::DONT_ENUM),
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to install lazy worker navigator"))?;
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_INSTALLED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    Ok(())
}

fn worker_navigator_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        throw_error(scope, "Worker navigator holder has no creation context.");
        return;
    };
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match ensure_worker_navigator_in_current_realm(target_scope) {
        Ok(navigator) => rv.set(navigator.into()),
        Err(error) => throw_error(
            target_scope,
            &format!("Failed to materialize WorkerNavigator: {error}"),
        ),
    }
}

fn ensure_worker_navigator_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(navigator) = get_private_value(scope, global, WINDOW_NAVIGATOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Ok(navigator);
    }
    if get_private_value(scope, global, WORKER_NAVIGATOR_MATERIALIZING_SLOT).is_some() {
        return Err(anyhow!("reentrant WorkerNavigator materialization"));
    }
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_MATERIALIZING_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let result = build_worker_navigator_in_current_realm(scope, global);
    set_private_value(
        scope,
        global,
        WORKER_NAVIGATOR_MATERIALIZING_SLOT,
        v8::undefined(scope).into(),
    );
    result
}

fn build_worker_navigator_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let user_agent = get_private_value(scope, global, WORKER_NAVIGATOR_USER_AGENT_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or_else(|| anyhow!("WorkerNavigator user-agent seed is missing"))?;
    let accept_language =
        get_private_value(scope, global, WORKER_NAVIGATOR_ACCEPT_LANGUAGE_SEED_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .ok_or_else(|| anyhow!("WorkerNavigator Accept-Language seed is missing"))?;
    let storage_apis_available =
        get_private_value(scope, global, WORKER_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT)
            .is_some_and(|value| value.boolean_value(scope));
    let identity = navigator_identity_profile(scope, global)
        .unwrap_or_else(|| BrowserIdentityProfile::new(user_agent, accept_language));
    let backing = build_worker_navigator_backing(scope, &identity)?;
    if let Some(prototype) = global_constructor_prototype(scope, "WorkerNavigator") {
        filter_worker_navigator_secure_context_exposure(scope, prototype, storage_apis_available)?;
    }
    let navigator = WorkerNavigatorObjectDeclaration::new(backing)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind WorkerNavigator object: {error}"))?;
    set_private_value(scope, global, WINDOW_NAVIGATOR_SLOT, navigator.into());
    Ok(navigator)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NavigatorStorageWrapperDiagnostics {
    pub(crate) navigator_materialized: bool,
    pub(crate) storage_manager_materialized: bool,
    pub(crate) storage_bucket_manager_materialized: bool,
}

#[cfg(test)]
pub(crate) fn navigator_storage_wrapper_diagnostics(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<NavigatorStorageWrapperDiagnostics> {
    let Some(navigator) = navigator_for_diagnostics(scope) else {
        return Some(NavigatorStorageWrapperDiagnostics {
            navigator_materialized: false,
            storage_manager_materialized: false,
            storage_bucket_manager_materialized: false,
        });
    };
    let backing = navigator_runtime_data_object(scope, navigator)?;
    let materialized = |scope: &mut v8::PinScope<'_, '_>, key: &'static str| {
        backing
            .get(scope, v8str(scope, key).into())
            .is_some_and(|value| !value.is_undefined())
    };
    Some(NavigatorStorageWrapperDiagnostics {
        navigator_materialized: true,
        storage_manager_materialized: materialized(scope, "storage"),
        storage_bucket_manager_materialized: materialized(scope, "storageBuckets"),
    })
}

#[cfg(test)]
pub(crate) fn materialized_navigator_subobject_keys(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<&'static str> {
    let Some(navigator) = navigator_for_diagnostics(scope) else {
        return Vec::new();
    };
    let Some(backing) = navigator_runtime_data_object(scope, navigator) else {
        return Vec::new();
    };
    NavigatorSubobject::ALL
        .into_iter()
        .filter_map(|subobject| {
            backing
                .get(scope, v8str(scope, subobject.key()).into())
                .is_some_and(|value| !value.is_undefined())
                .then_some(subobject.key())
        })
        .collect()
}

#[cfg(test)]
fn navigator_for_diagnostics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WINDOW_NAVIGATOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn navigator_runtime_data_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigator: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, navigator, NAVIGATOR_RUNTIME_DATA_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

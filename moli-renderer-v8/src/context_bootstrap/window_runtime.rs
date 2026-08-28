use super::*;

mod base64;
mod date_locale;
mod dialogs;
mod lifecycle;
mod navigator;
mod performance;
mod service_worker;
mod structured_clone;
mod window_features;

use moli_webapi_declare::WebApiObject;

pub(super) use base64::{window_atob_callback, window_btoa_callback};
pub(super) use date_locale::{
    current_date_locale_overrides, date_to_locale_date_string_callback,
    date_to_locale_string_callback, date_to_locale_time_string_callback,
};
pub(crate) use date_locale::{
    set_date_locale_override_for_current_context, set_date_timezone_override_for_current_context,
};
pub(super) use dialogs::entered_window_api_base_url;
pub(super) use dialogs::{window_alert_callback, window_confirm_callback, window_prompt_callback};
pub(crate) use dialogs::{
    window_const_false_callback, window_noop_callback, window_open_callback, window_stop_callback,
};
pub(crate) use lifecycle::{
    accept_remote_top_level_browsing_context_focus,
    dispatch_top_level_browsing_context_close_unload, request_top_level_browsing_context_close,
    request_top_level_browsing_context_focus,
};
pub(super) use lifecycle::{window_close_callback, window_closed_getter, window_focus_callback};
pub(crate) use navigator::{
    LegacyStorageQuotaCallbackOutcome, LegacyStorageQuotaCallbackTask,
    LegacyStorageQuotaCallbackTaskEffect,
};
pub(super) use navigator::{
    MEDIA_DEVICES_BRAND_SLOT, PERMISSIONS_BRAND_SLOT, build_legacy_storage_info_object,
    build_legacy_storage_quota_object, build_navigator_ua_data_object,
    global_caches_getter_callback, navigator_get_battery_callback, navigator_java_enabled_callback,
    navigator_media_devices_enumerate_devices_callback,
    navigator_media_devices_get_user_media_callback, navigator_permissions_query_callback,
    navigator_send_beacon_callback, navigator_storage_estimate_callback,
    navigator_storage_get_directory_callback, navigator_storage_persist_callback,
    navigator_storage_persisted_callback, navigator_ua_data_get_high_entropy_values_callback,
    navigator_ua_data_to_json_callback, navigator_vibrate_callback,
    permission_status_name_getter_callback, permission_status_state_getter_callback,
    storage_bucket_caches_getter_callback, storage_bucket_durability_callback,
    storage_bucket_estimate_callback, storage_bucket_expires_callback,
    storage_bucket_get_directory_callback, storage_bucket_indexed_db_getter_callback,
    storage_bucket_manager_delete_callback, storage_bucket_manager_keys_callback,
    storage_bucket_manager_open_callback, storage_bucket_name_getter_callback,
    storage_bucket_persist_callback, storage_bucket_persisted_callback,
    storage_bucket_set_expires_callback,
};
pub(super) use performance::performance_now_callback;
pub(crate) use service_worker::{
    ServiceWorkerClientMessageCallbackDispatchEffect, ServiceWorkerClientMessageDispatchEffect,
    ServiceWorkerInternalEventCallbackDispatchEffect, dispatch_service_worker_client_message_body,
    dispatch_service_worker_controller_change, dispatch_service_worker_lifecycle_notification,
    settle_service_worker_ready_completion, settle_service_worker_register_completion,
    settle_service_worker_unregister_completion,
};
pub(super) use service_worker::{
    install_initial_service_worker_ready_promise,
    navigator_service_worker_controller_getter_callback,
    navigator_service_worker_controllerchange_handler_getter_callback,
    navigator_service_worker_controllerchange_handler_setter_callback,
    navigator_service_worker_get_registration_callback,
    navigator_service_worker_get_registrations_callback,
    navigator_service_worker_message_handler_getter_callback,
    navigator_service_worker_message_handler_setter_callback,
    navigator_service_worker_messageerror_handler_getter_callback,
    navigator_service_worker_messageerror_handler_setter_callback,
    navigator_service_worker_register_callback, service_worker_object_set_owner_scope,
};
pub(super) use structured_clone::window_structured_clone_callback;

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ChildWindowOwnMethodsDeclaration {
    #[webapi(method, length = 0, callback = window_open_callback)]
    open: (),
    #[webapi(method, length = 0, callback = window_noop_callback)]
    close: (),
    #[webapi(method, length = 0, callback = window_noop_callback)]
    blur: (),
    #[webapi(method, length = 0, callback = window_const_false_callback)]
    find: (),
    #[webapi(method, length = 0, callback = window_stop_callback)]
    stop: (),
    #[webapi(method, length = 0, callback = window_noop_callback)]
    print: (),
}

pub(crate) fn install_child_window_own_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), moli_webapi_declare::BindError> {
    ChildWindowOwnMethodsDeclaration::default().initialize(scope, window)
}

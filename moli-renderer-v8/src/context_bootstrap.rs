mod animation_runtime;
mod assets;
pub(crate) mod bridge_descriptor;
mod broadcast_channel;
mod canvas;
mod constructors;
mod crypto;
mod css_fontface_runtime;
mod css_runtime;
pub(crate) mod css_stylesheet_runtime;
mod date_locale_runtime;
mod dom_rect;
mod event_document;
mod event_legacy;
mod event_template;
mod events;
pub(crate) mod exposed_interfaces;
mod file_api;
mod form_data_runtime;
mod geometry_runtime;
mod history_mutation;
mod history_runtime;
mod idle_detection;
mod image_data;
pub(crate) use self::idle_detection::apply_idle_override_to_current_context;
mod indexed_db;
#[cfg(test)]
pub(crate) use exposed_interfaces::{
    interface_materialization_count as lazy_constructor_materialization_count,
    interface_template_build_count as lazy_constructor_template_build_count,
    lazy_window_interface_names,
    materialized_interface_names as lazy_materialized_constructor_names,
    ready_interface_template_names as lazy_ready_constructor_template_names,
    storage_interface_materialization_count as lazy_storage_constructor_materialization_count,
};
mod location_history_storage;
mod location_navigation;
mod location_runtime;
mod media_cues;
mod media_file_template;
mod media_queries;
mod media_source;
mod message_ports;
mod microtask_checkpoint;
mod navigation_activation;
mod navigation_bootstrap;
mod navigation_callbacks;
mod navigation_cancellation;
mod navigation_cross_document;
mod navigation_entry;
mod navigation_entry_state;
mod navigation_events;
mod navigation_handler_callbacks;
mod navigation_history_pruning;
mod navigation_lifecycle;
mod navigation_mutation;
mod navigation_projection;
mod navigation_restore;
mod navigation_result;
mod navigation_seed;
mod navigation_serialize;
mod navigation_surface;
mod navigation_traversal;
mod navigation_traversal_execution;
mod navigation_traversal_plan;
mod navigation_window;
mod navigator_runtime;
#[cfg(test)]
pub(crate) use navigator_runtime::{
    materialized_navigator_subobject_keys, navigator_storage_wrapper_diagnostics,
};
mod notification_runtime;
mod observer_template;
mod opfs;
pub(crate) use self::opfs::{
    FileSystemFileSnapshotClonePayload, FileSystemHandleClonePayload,
    FileSystemHandleDurablePayload, attach_file_system_file_snapshot_clone_payload,
    build_file_system_handle_from_clone_payload, build_file_system_handle_from_durable_payload,
    file_system_file_snapshot_clone_payload_from_object,
    file_system_handle_clone_payload_from_object, file_system_handle_durable_payload_from_object,
    settle_opfs_directory_iterator_task_result, settle_opfs_move_task_result,
    settle_opfs_task_result,
};
mod performance_observer_runtime;
mod performance_runtime;
mod range;
mod range_algorithms;
mod range_live;
mod range_surface;
mod resize_observer_runtime;
pub(crate) use resize_observer_runtime::queue_resize_observer_checks;
mod runtime_state;
mod selection;
mod selection_callbacks;
mod selection_modify;
mod selection_surface;
mod shared;
mod shared_installers;
mod shared_worker_host;
mod speech_synthesis;
mod storage_access;
mod storage_buckets;

pub(crate) use self::storage_access::request_storage_access_with_types;
pub(crate) use self::window_runtime::{
    LegacyStorageQuotaCallbackOutcome, LegacyStorageQuotaCallbackTask,
    LegacyStorageQuotaCallbackTaskEffect,
};

pub use moli_storage_service::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES;

#[cfg(test)]
pub(crate) use self::css_runtime::css_lazy_state_diagnostics;
pub(crate) use self::css_runtime::{
    css_supports_condition_text, install_css_runtime_state_for_document,
};
pub(crate) use self::events::{
    construct_original_event, construct_original_page_transition_event,
    construct_original_storage_event_utf16,
};
pub(crate) use crypto::{
    CryptoKeyAlgorithmClonePayload, CryptoKeyClonePayload, WebCryptoRejection, WebCryptoTaskResult,
    crypto_key_clone_payload_from_object, crypto_key_object_from_clone_payload,
    is_crypto_key_object,
};
pub(crate) use css_fontface_runtime::rebuild_font_face_set_faces;
pub(crate) use location_navigation::{
    LocationNavigationKind, dispatch_top_level_form_navigation_event,
    dispatch_top_level_navigation_event_with_source_element, meta_refresh_navigation_kind,
    navigate_location_object_with_child_navigate_event,
    navigate_location_object_with_source_element, navigate_top_level_meta_refresh,
    navigate_top_level_same_document_from_browser,
};
pub(crate) use navigation_cancellation::inform_about_canceled_navigation_for_window;
pub(crate) use navigation_events::dispatch_cross_document_navigation_navigate_event_for_window_with_form_data;
pub(crate) use navigation_events::{
    construct_original_hash_change_event, dispatch_beforeunload_for_runtime_owner,
    dispatch_pagehide_for_runtime_owner, dispatch_unload_for_runtime_owner,
};
pub(crate) use navigation_history_pruning::{
    NavigationHistoryPrunePlan, apply_navigation_history_prune_plan,
    finalize_navigation_history_prune, plan_navigation_history_prune,
};
pub(crate) use navigation_result::{
    NavigationFinishedResultApplication, apply_pending_navigation_finished_result,
};
pub(crate) use navigation_traversal_execution::apply_authorized_history_traversal_task;
pub(crate) use performance_runtime::PERFORMANCE_TIME_ORIGIN_SLOT;
pub(crate) use performance_runtime::performance_slot_number;
pub(crate) use selection_surface::{
    selection_value_for_window, sync_selection_owner_document_for_window,
};
pub(crate) use shared::{dom_time_since_origin_millis, unix_epoch_millis};
pub(crate) use window_runtime::{
    set_date_locale_override_for_current_context, set_date_timezone_override_for_current_context,
};
mod specs;
mod stream_adapter;
mod stream_objects;
mod streams;
mod style_font_template;
pub(crate) mod svg_runtime;
mod touch_runtime;
mod trusted_types;
mod url_form;
mod url_search_params_runtime;
mod view_transition_runtime;
mod web_audio_runtime;
mod web_storage;
mod webassembly_runtime;
mod webrtc;
mod websocket;
mod window_accessors;
mod window_events;
mod window_lazy_surface;
mod window_receiver;
pub(crate) use window_receiver::is_window_receiver;
mod window_runtime;

pub(crate) use form_data_runtime::{
    construct_form_data_entries_for_form, form_data_entries_multipart_body_with_prefix,
    form_data_entries_to_string_pairs, form_data_object_from_entries,
    form_data_object_from_multipart_bytes, form_data_object_from_urlencoded_bytes,
    snapshot_form_data_value,
};
pub(crate) use view_transition_runtime::run_view_transition_update_callback;
pub(crate) use webassembly_runtime::{
    mark_module_instantiation_exceeds_v8_limit, module_instantiation_exceeds_v8_limit,
};
pub(crate) use window_runtime::{
    ServiceWorkerClientMessageCallbackDispatchEffect, ServiceWorkerClientMessageDispatchEffect,
    ServiceWorkerInternalEventCallbackDispatchEffect, dispatch_service_worker_client_message_body,
    dispatch_service_worker_controller_change, dispatch_service_worker_lifecycle_notification,
    settle_service_worker_ready_completion, settle_service_worker_register_completion,
    settle_service_worker_unregister_completion,
};
mod window_template;
mod worker_host;
mod worker_location_runtime;

pub(super) use self::assets::ContextBootstrapAssets;
use self::assets::{build_constructor_template, build_constructor_template_with_callback};
pub(crate) use self::broadcast_channel::{
    dispatch_authorized_page_broadcast_channel_event, dispatch_broadcast_channel_events_for_channel,
};
pub(crate) use self::canvas::{
    CanvasContextKind, attach_canvas_like_context_object, build_canvas_rendering_context_2d_object,
    build_offscreen_canvas_object, build_webgl_context_object, build_webgl2_context_object,
    canvas_like_to_data_url, reset_html_canvas_backing_store_for_dimension_assignment,
};
#[cfg(test)]
pub(crate) use self::constructors::finalize_dom_exception_realm_bindings;
use self::constructors::illegal_constructor_callback;
pub(crate) use self::constructors::{
    dom_exception_clone_fields, ensure_dom_implementation_singleton, new_dom_error_value,
    new_dom_exception_value, new_most_derived_dom_exception_value, new_quota_exceeded_error_value,
    quota_exceeded_error_clone_fields, throw_dom_exception_value,
};
pub(crate) use self::css_stylesheet_runtime::{
    adopted_style_sheet_installations_from_value, bind_css_style_sheet_to_live_stylesheet,
    clear_css_style_sheet_document_adopted_owner_tracking, clear_css_style_sheet_owner_node,
    clear_css_style_sheet_shadow_root_adopted_owner_tracking,
    css_style_sheet_constructor_document_handle, css_style_sheet_id,
    css_style_sheet_is_constructed, initialize_css_module_style_sheet_object,
    initialize_css_style_sheet_rules_from_text, new_css_style_sheet_object,
    new_style_sheet_list_object, set_css_style_sheet_href, set_css_style_sheet_origin_clean,
    set_css_style_sheet_owner_node, set_style_sheet_list_contents,
    sync_constructed_css_style_sheet_rules_from_text,
    sync_css_style_sheet_document_adopted_owner_tracking,
    sync_css_style_sheet_media_list_from_owner,
    sync_css_style_sheet_shadow_root_adopted_owner_tracking,
};
pub(crate) use self::dom_rect::build_dom_rect_object;
pub(crate) use self::events::{
    EVENT_DISPATCHING_SLOT, EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT,
    EVENT_STOP_PROPAGATION_SLOT, clear_event_composed_path, event_initialized,
    event_internal_bool_flag, event_is_dispatching, event_is_error_event, event_is_mouse_event,
    initialize_event_object, mark_event_trusted, set_event_composed_path, set_event_internal_flag,
    set_event_trusted,
};
pub(crate) use self::file_api::{
    DataTransferStringCallbackTask, DataTransferStringCallbackTaskEffect,
    DirectoryReaderCallbackAdmission, DirectoryReaderCallbackTask,
    DirectoryReaderCallbackTaskEffect, FileEntryFileCallbackTask, FileEntryFileCallbackTaskEffect,
};
pub(crate) use self::file_api::{apply_drag_modifier_drop_effect, build_data_transfer_object};
pub(crate) use self::file_api::{
    build_file_list_object, build_file_object, flush_one_pending_file_reader,
    selected_file_from_object,
};
pub(crate) use self::form_data_runtime::form_data_request_body;
use self::geometry_runtime::{build_dom_point_object, optional_dom_point_init_arg};
pub(crate) use self::history_runtime::{
    increment_top_level_history_length_for_runtime_owner,
    set_top_level_history_length_at_least_for_runtime_owner,
};
pub(crate) use self::image_data::{
    ImageDataClonePayload, build_image_data_object_from_clone_payload,
    image_data_clone_payload_from_object, is_image_data_object,
};
pub(crate) use self::indexed_db::{
    IndexedDbTaskId, discard_indexed_db_task_by_id, flush_blocked_indexed_db_requests,
    flush_indexed_db_task_by_id, flush_next_indexed_db_task, indexed_db_has_pending_tasks,
    install_worker_indexed_db_runtime_state, set_indexed_db_manager_for_context,
    set_worker_indexed_db_task_wake_for_context,
};
pub use self::indexed_db::{
    Key as IndexedDbKey, ObjectStoreOptions as IndexedDbObjectStoreOptions,
    OpenOptions as IndexedDbOpenOptions, SharedIndexedDbManager,
    TransactionMode as IndexedDbTransactionMode, WeakIndexedDbManager, clear_indexed_db_origin,
    clear_indexed_db_origins_with_prefix, downgrade_indexed_db_manager,
    indexed_db_origin_usage_bytes, indexed_db_origins_with_prefix_usage_bytes,
    new_indexed_db_manager,
};
pub(crate) use self::indexed_db::{
    bind_indexed_db_factory_to_window_execution_context,
    materialized_indexed_db_factory_for_window, scoped_indexed_db_factory,
};
#[cfg(test)]
pub(crate) use self::indexed_db::{
    indexed_db_manager_context_slot_present_for_test,
    indexed_db_manager_isolate_slot_present_for_test,
};
pub(in crate::context_bootstrap) use self::indexed_db::{
    indexed_db_usage_bytes_for_storage_key, scoped_storage_bucket_indexed_db_factory,
};
pub(crate) use self::location_runtime::sync_global_location_runtime_state;
pub(crate) use self::location_runtime::{
    sync_document_location_runtime_state_from_window,
    sync_window_location_history_navigation_runtime_surface, sync_window_location_runtime_state,
};
pub(crate) use self::media_cues::set_text_track_cue_track;
pub(crate) use self::media_queries::{
    SimpleObjectEventListenerInspectorSnapshot, SimpleObjectEventListenerSnapshot,
    dispatch_media_query_list_change_events, dispatch_simple_event_target_event,
    evaluate_match_media_query_list_with_viewport, install_simple_event_target_methods,
    install_simple_event_target_ordered_handlers, invoke_simple_event_listener,
    mark_simple_event_target_slot, simple_event_target_add_event_listener_callback,
    simple_event_target_dispatch_event_callback, simple_event_target_inspector_listener_snapshots,
    simple_event_target_remove_event_listener_callback, simple_event_target_slot_name,
    simple_object_event_listeners_snapshot, simple_object_event_remove_listener_value_for_type,
    simple_object_event_set_ordered_handler, simple_object_event_target_add_listener,
    simple_object_event_target_remove_listener,
};
use self::message_ports::schedule_host_callback;
pub(crate) use self::message_ports::{
    MessagePortDeliveryRunResult, MessagePortEventListenerId, MessagePortEventListenerSnapshot,
    PreparedMessagePortEventListener, PreparedMessagePortEventListenerCallback,
    WindowMessagePortEventListenerRegistry, WorkerMessagePortEventListenerRegistry,
    current_message_port_owner, detach_message_port_owner_for_transfer,
    detach_transferred_message_port, dispatch_message_port_events_for_port_collecting_errors,
    dispatch_one_authorized_message_port_event, ensure_message_port_wrapper_for_id,
    message_port_id_from_object,
};
pub(crate) use self::microtask_checkpoint::{
    install_agent_microtask_checkpoint_tasks, run_end_of_microtask_checkpoint_tasks,
};
pub(crate) use self::navigation_bootstrap::{
    install_window_location_history_navigation_runtime_state,
    reset_window_location_history_navigation_runtime_state,
};
pub(crate) use self::navigation_events::dispatch_cross_document_navigation_navigate_event_for_window;
pub(crate) use self::navigation_events::dispatch_srcdoc_navigation_navigate_event_for_window;
pub(crate) use self::navigation_mutation::apply_local_window_location_navigation;
pub(crate) use self::navigation_restore::{
    install_navigation_bootstrap_entry, install_navigation_bootstrap_entry_for_holder,
};
pub(crate) use self::navigation_traversal::queue_top_level_history_traversal_by_delta;
pub(crate) use self::navigator_runtime::install_worker_navigator_runtime_state;
pub(crate) use self::navigator_runtime::{
    bind_window_navigator_identity_seed, set_window_navigator_identity,
    update_cached_window_visual_viewport_dimensions,
};
pub(crate) use self::notification_runtime::{
    build_notification_object_from_snapshot, notification_get_options_tag,
    notification_options_payload,
};
pub(crate) use self::performance_runtime::{
    ResourcePerformanceEntry, bind_window_performance_seed, current_performance_time_origin,
    increment_performance_event_count, record_performance_dom_content_loaded_event_end,
    record_performance_dom_content_loaded_event_start, record_performance_load_event_end,
    record_performance_load_event_start, record_resource_performance_entry,
    run_resource_timing_buffer_full_task,
};
use self::range::callback_arg_node_object;
use self::range_live::{
    update_live_ranges_for_character_data_edit, update_live_ranges_for_character_data_reset,
    update_live_ranges_for_child_insertion, update_live_ranges_for_child_removal,
    update_live_ranges_for_detached_character_data_edit,
    update_live_ranges_for_detached_character_data_reset,
    update_live_ranges_for_detached_child_insertion, update_live_ranges_for_detached_text_split,
    update_live_ranges_for_text_split,
};
pub(super) use self::runtime_state::finish_context_bootstrap;
pub(crate) use self::runtime_state::install_child_window_eval_runtime_state;
pub(crate) use self::runtime_state::install_webassembly_runtime_state;
#[cfg(feature = "wpt-extensions")]
pub(crate) use self::runtime_state::install_wpt_webdriver_runtime_state;
pub(crate) use self::runtime_state::set_window_origin_runtime_state;
pub(crate) use self::runtime_state::{
    ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_GLOBAL_VALUE_GETTER_SLOT, ORIGINAL_WEBASSEMBLY_INSTANCE_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
};
#[cfg(test)]
pub(crate) use self::shared::structured_deserialize_value;
#[cfg(test)]
pub(crate) use self::shared::structured_serialize_value;
pub(in crate::context_bootstrap) use self::shared::*;
pub(crate) use self::shared::{
    CHILD_BROWSING_CONTEXT_HANDLE_SLOT, DOCUMENT_SELECTION_CHANGE_LISTENER_SLOT,
    READABLE_STREAM_CHILD_REALM_HANDLED_REJECTION_SLOT, WINDOW_CUSTOM_ELEMENTS_SLOT,
    WINDOW_NAME_SLOT,
};
pub(crate) use self::shared::{
    RuntimeMessageSourceSecurity, current_runtime_message_agent_cluster,
    runtime_message_allowed_for_current_target, structured_clone_value,
    structured_clone_value_with_options, structured_deserialize_value_for_message_event,
    structured_serialize_value_for_post_message,
    structured_serialize_value_for_post_message_with_source_port,
    structured_serialize_value_for_window_post_message,
    structured_serialize_value_for_window_post_message_options,
    wasm_module_message_allowed_for_target, wasm_module_message_allowed_for_target_origin,
};
pub(crate) use self::shared::{
    SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT,
};
pub(crate) use self::shared::{
    console_arg_remote_object_json, current_console_stack,
    install_console_message_buffers_for_context,
    snapshot_console_message_details_for_current_context,
    snapshot_console_messages_for_current_context,
};
pub(crate) use self::shared_worker_host::dispatch_shared_worker_client_error;
use self::specs::constructor_specs;
pub(crate) use self::storage_buckets::set_storage_bucket_store_for_context;
pub use self::storage_buckets::{
    SharedStorageBucketStore, StorageBucketIdentity, new_shared_json_storage_bucket_store,
    new_shared_json_storage_bucket_store_with_cache_root,
    new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager,
    new_shared_json_storage_bucket_store_with_storage_service, new_shared_storage_bucket_store,
    new_shared_storage_bucket_store_with_indexed_db_manager,
    new_shared_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
    storage_bucket_indexed_db_storage_key,
};
pub(crate) use self::stream_adapter::{
    cancel_readable_stream, close_stream, enqueue_byte_chunk, error_stream,
    readable_stream_disturbed, readable_stream_has_pipe_owner, require_internal_stream_value,
};
pub(crate) use self::streams::{
    ReadableStreamClonePayload, TransformStreamClonePayload, WritableStreamClonePayload,
    build_readable_stream_clone_shell, build_transform_stream_clone_shell,
    build_writable_stream_clone_shell, initialize_readable_stream_clone_shell,
    initialize_transform_stream_clone_shell, initialize_writable_stream_clone_shell,
    is_readable_stream_object, is_transform_stream_object, is_writable_stream_object,
    new_readable_stream_from_array_buffer, new_readable_stream_from_source,
    prepare_readable_stream_transfer, prepare_transform_stream_transfer,
    prepare_writable_stream_transfer,
};
#[cfg(test)]
pub(crate) use self::trusted_types::trusted_types_lazy_state_materialized;
pub(crate) use self::trusted_types::{
    TrustedTypesCodeGenerationCheck, install_trusted_types_eval_runtime_state,
    install_trusted_types_runtime_state, trusted_html_string_or_throw, trusted_html_value_string,
    trusted_script_string_for_script_element_execution, trusted_script_string_or_type_error,
    trusted_script_url_string_or_throw, trusted_types_code_generation_check,
    trusted_types_code_generation_check_callback,
};
pub(crate) use self::url_form::object_prototype_matches;
pub(crate) use self::url_search_params_runtime::url_search_params_request_body;
pub(crate) use self::web_storage::install_storage_aliases_for_window;
pub use self::web_storage::{
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord,
    WebStorageMutationSubscription, WebStorageString, deep_clone_shared_web_storage_store,
    new_shared_json_web_storage_store, new_shared_web_storage_store,
    web_storage_area_key_for_storage_key, web_storage_partitioned_area_key,
};
pub(crate) use self::webassembly_runtime::{
    set_current_context_webassembly_default_prototype, webassembly_default_prototype_for_context,
};
pub(crate) use self::websocket::{WebSocketDispatchResult, dispatch_websocket_event};
pub(crate) use self::window_events::{
    WINDOW_EVENT_HANDLER_PROPERTIES, dispatch_window_error_event_with_details,
    dispatch_window_promise_rejection_event, dispatch_window_report_error_message,
    set_window_body_onerror_handler_compiled, set_window_onerror_handler_value,
    window_body_onerror_handler_is_compiled,
};
#[cfg(test)]
pub(crate) use self::window_lazy_surface::window_lazy_surface_diagnostics;
pub(crate) use self::window_lazy_surface::{
    WindowLazySurface, ensure_window_lazy_surface_object,
    rematerialize_window_lazy_surface_if_cached,
};
use self::window_runtime::global_caches_getter_callback;
pub(crate) use self::window_runtime::install_child_window_own_methods;
pub(crate) use self::window_template::install_window_own_template_bindings;
pub(crate) use self::worker_host::{
    dispatch_worker_error_event_with_error, dispatch_worker_error_event_with_kind,
    dispatch_worker_event, flush_pending_worker_messages_for_listener,
    worker_has_message_delivery_listener,
};
pub(crate) use self::worker_location_runtime::install_worker_location_runtime_state;
pub(super) use super::{
    blob,
    native_bridge::{self, JsContextHost},
    util::{
        callback_data_item, context_host_ptr_from_global_bridge, get_private_value,
        object_number_property, set_private_value, throw_range_error, throw_type_error, v8_string,
        v8str,
    },
};
use anyhow::{Result, anyhow};
pub(crate) use exposed_interfaces::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const WORKER_CURRENT_SCRIPT_URL_SLOT: &str = "__moliWorkerCurrentScriptUrl";

pub(crate) fn current_child_frame_id_for_runtime_scope(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> Option<String> {
    let global = scope.get_current_context().global(scope);
    let owner = self::navigation_window::runtime_window_owner(scope, global);
    let handle =
        self::navigation_window::child_browsing_context_handle_for_runtime_owner(scope, owner)?;
    host.child_browsing_context_request_scope(handle)
        .map(|(frame_id, _)| frame_id)
}

pub(crate) fn current_child_browsing_context_handle_for_runtime_scope(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::document_runtime::DomHandle> {
    if let Some(handle) = native_bridge::active_child_window_handle(scope) {
        return Some(handle);
    }
    child_browsing_context_handle_for_current_realm_scope(scope)
}

pub(crate) fn child_browsing_context_handle_for_current_realm_scope(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::document_runtime::DomHandle> {
    let global = scope.get_current_context().global(scope);
    if let Some(handle) = get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| self::navigation_window::dom_handle_from_marker_value(scope, value))
    {
        return Some(handle);
    }
    let key = v8str(scope, CHILD_BROWSING_CONTEXT_HANDLE_SLOT);
    if let Some(handle) = global
        .get(scope, key.into())
        .and_then(|value| self::navigation_window::dom_handle_from_marker_value(scope, value))
    {
        return Some(handle);
    }
    let owner = self::navigation_window::runtime_window_owner(scope, global);
    self::navigation_window::child_browsing_context_handle_for_runtime_owner(scope, owner)
}

pub(crate) fn build_lightweight_popup_window_navigator_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_popup: u64,
) -> Result<v8::Local<'s, v8::Object>> {
    navigator_runtime::build_lightweight_popup_window_navigator_object(scope, owner_popup)
}

fn find_constructor_spec(name: &str) -> Result<self::specs::ConstructorSpec> {
    constructor_specs()
        .into_iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| anyhow!("missing context bootstrap constructor spec `{name}`"))
}

#[cfg(test)]
pub(crate) fn build_named_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    name: &str,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    build_constructor_template(scope, find_constructor_spec(name)?)
}

/// Installs a Worker interface which still owns bootstrap-time concrete state
/// outside the shared lazy template registry.
fn install_worker_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> Result<()> {
    let template = build_constructor_template(scope, find_constructor_spec(name)?)?;
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to instantiate eager worker constructor `{name}`"))?;
    define_global_value(scope, global, name, constructor.into())?;
    install_to_string_tag(scope, global, name, name);
    Ok(())
}

pub(crate) fn install_worker_lazy_exposed_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    realm_kind: exposed_interfaces::RealmKind,
    secure_context: bool,
) -> Result<()> {
    let specs = constructor_specs();
    exposed_interfaces::install_worker_exposed_interfaces(
        scope,
        global,
        realm_kind,
        secure_context,
        specs,
    )?;
    if !secure_context {
        return Ok(());
    }
    GlobalCachesAccessorDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker caches accessor: {error}"))
}

pub(in crate::context_bootstrap) fn build_profiled_exposed_interface_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    spec: self::specs::ConstructorSpec,
    profile: exposed_interfaces::TemplateBuildProfile,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    if profile == exposed_interfaces::TemplateBuildProfile::Window {
        let template = build_constructor_template(scope, spec)?;
        if spec.name == "XMLHttpRequest" {
            crate::network_host::install_window_xml_http_request_template_bindings(scope, template);
        }
        return Ok(template);
    }
    let template = match spec.name {
        "StorageManager" => {
            let template = navigator_runtime::build_storage_manager_worker_template(scope);
            template.read_only_prototype();
            exposed_interfaces::install_interface_template_metadata(
                scope,
                template,
                "StorageManager",
            );
            template
        }
        "AbortSignal" => {
            let template = WorkerAbortSignalTemplateDeclaration::build(scope);
            template.read_only_prototype();
            exposed_interfaces::install_interface_template_metadata(scope, template, spec.name);
            template
        }
        "AbortController" => {
            let template = WorkerAbortControllerTemplateDeclaration::build(scope);
            template.read_only_prototype();
            exposed_interfaces::install_interface_template_metadata(scope, template, spec.name);
            template
        }
        "EventSource" => build_constructor_template_with_callback(
            scope,
            spec,
            worker_unsupported_constructor_callback,
        )?,
        _ => build_constructor_template(scope, spec)?,
    };
    if profile == exposed_interfaces::TemplateBuildProfile::DedicatedWorker
        && spec.name == "FileSystemFileHandle"
    {
        opfs::install_file_system_file_handle_sync_template_binding(scope, template);
    }
    Ok(template)
}

fn worker_unsupported_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let exception = crate::native_bridge::abort::dom_exception_value(
        scope,
        "This constructor is not implemented in dedicated workers yet.",
        "NotSupportedError",
    );
    scope.throw_exception(exception);
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
pub(in crate::context_bootstrap) struct GlobalCachesAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = global_caches_getter_callback,
        enumerable
    )]
    caches: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerBase64OperationsDeclaration {
    #[webapi(
        method,
        callback = self::window_runtime::window_btoa_callback,
        length = 1
    )]
    btoa: (),
    #[webapi(
        method,
        callback = self::window_runtime::window_atob_callback,
        length = 1
    )]
    atob: (),
}

pub(crate) fn install_worker_base64_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    WorkerBase64OperationsDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker base64 operations: {error}"))
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AbortSignal", enumerable)]
struct WorkerAbortSignalTemplateDeclaration {
    #[webapi(
        static_method = "abort",
        length = 1,
        callback = crate::worker::abort::worker_abort_signal_static_abort_callback
    )]
    abort_static: (),

    #[webapi(
        static_method = "timeout",
        length = 1,
        callback = crate::worker::abort::worker_abort_signal_timeout_callback
    )]
    timeout: (),

    #[webapi(
        static_method = "any",
        length = 1,
        callback = crate::worker::abort::worker_abort_signal_any_callback
    )]
    any: (),

    #[webapi(
        method = "addEventListener",
        length = 2,
        callback = crate::worker::abort::worker_abort_signal_add_event_listener_callback
    )]
    add_event_listener: (),

    #[webapi(
        method = "removeEventListener",
        length = 2,
        callback = crate::worker::abort::worker_abort_signal_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(
        method = "dispatchEvent",
        length = 1,
        callback = crate::worker::abort::worker_abort_signal_dispatch_event_callback
    )]
    dispatch_event: (),

    #[webapi(
        method = "throwIfAborted",
        length = 0,
        callback = crate::worker::abort::worker_abort_signal_throw_if_aborted_callback
    )]
    throw_if_aborted: (),

    #[webapi(accessor_property, getter = crate::worker::abort::worker_abort_signal_aborted_getter_function)]
    aborted: (),

    #[webapi(accessor_property, getter = crate::worker::abort::worker_abort_signal_reason_getter_function)]
    reason: (),

    #[webapi(
        accessor_property,
        getter = crate::worker::abort::worker_abort_signal_onabort_getter_function,
        setter = crate::worker::abort::worker_abort_signal_onabort_setter_function
    )]
    onabort: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "AbortController",
    constructor_callback = crate::worker::abort::worker_abort_controller_constructor_callback,
    constructor_length = 0,
    enumerable
)]
struct WorkerAbortControllerTemplateDeclaration {
    #[webapi(
        accessor_property,
        getter = crate::worker::abort::worker_abort_controller_signal_getter
    )]
    signal: (),

    #[webapi(
        method = "abort",
        length = 1,
        callback = crate::worker::abort::worker_abort_controller_abort_callback
    )]
    abort: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "WorkerGlobalScope", enumerable)]
struct WorkerGlobalScopeCryptoPrototypeDeclaration {
    #[webapi(accessor_property, getter = worker_crypto_getter_callback)]
    crypto: (),
}

pub(crate) fn initialize_worker_fetch_realm_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    crate::network_host::initialize_fetch_realm_helpers(scope)?;
    Ok(())
}

pub(crate) fn initialize_worker_crypto_realm_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    subtle_crypto_available: bool,
) -> Result<()> {
    self::crypto::install_worker_crypto_runtime_state(scope, global, subtle_crypto_available)?;
    install_worker_crypto_global_attribute(scope, global)?;
    Ok(())
}

fn install_worker_crypto_global_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(prototype) = global_constructor_prototype(scope, "WorkerGlobalScope") else {
        // Some unit tests install only the worker crypto surface in a bare V8
        // context. Real dedicated workers install WorkerGlobalScope first and
        // take the WebIDL prototype-attribute path below.
        return global
            .set_lazy_data_property_with_configuration(
                scope,
                v8str(scope, "crypto").into(),
                v8::LazyDataPropertyConfiguration::new(worker_crypto_lazy_getter_callback)
                    .property_attribute(v8::PropertyAttribute::DONT_ENUM),
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| anyhow!("failed to install lazy worker crypto property"));
    };
    WorkerGlobalScopeCryptoPrototypeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize WorkerGlobalScope crypto: {error}"))
}

fn worker_crypto_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    if !args.this().strict_equals(global.into()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    match self::crypto::ensure_worker_crypto_for_global(scope, global) {
        Ok(crypto) => rv.set(crypto.into()),
        Err(error) => throw_error(
            scope,
            &format!("Failed to materialize worker crypto: {error}"),
        ),
    }
}

fn worker_crypto_lazy_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        throw_error(scope, "Worker crypto holder has no creation context.");
        return;
    };
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let global = relevant_context.global(target_scope);
    match self::crypto::ensure_worker_crypto_for_global(target_scope, global) {
        Ok(crypto) => rv.set(crypto.into()),
        Err(error) => throw_error(
            target_scope,
            &format!("Failed to materialize worker crypto: {error}"),
        ),
    }
}

pub(crate) fn initialize_worker_file_realm_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    self::file_api::initialize_file_api_runtime_queues(scope, global)?;
    Ok(())
}

pub(crate) fn install_worker_script_url_runtime_state(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    script_url: &url::Url,
) -> Result<()> {
    let url = v8_string(scope, script_url.as_str())
        .ok_or_else(|| anyhow!("failed to allocate worker script url"))?;
    set_private_value(scope, global, WORKER_CURRENT_SCRIPT_URL_SLOT, url.into());
    Ok(())
}

pub(crate) fn current_worker_script_url(scope: &mut v8::PinScope<'_, '_>) -> Option<url::Url> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WORKER_CURRENT_SCRIPT_URL_SLOT)
        .and_then(|value| value.to_string(scope))
        .and_then(|value| url::Url::parse(&value.to_rust_string_lossy(scope)).ok())
}

pub(crate) fn live_ranges_character_data_edit(
    scope: &mut v8::PinScope<'_, '_>,
    target: super::document_runtime::DomHandle,
    edit_offset: u32,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_character_data_edit(
        scope,
        target,
        edit_offset,
        removed_count,
        inserted_count,
    );
}

pub(crate) fn live_ranges_character_data_reset(
    scope: &mut v8::PinScope<'_, '_>,
    target: super::document_runtime::DomHandle,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_character_data_reset(scope, target, removed_count, inserted_count);
}

pub(crate) fn live_ranges_detached_character_data_edit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    edit_offset: u32,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_detached_character_data_edit(
        scope,
        target,
        edit_offset,
        removed_count,
        inserted_count,
    );
}

pub(crate) fn live_ranges_detached_character_data_reset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_detached_character_data_reset(
        scope,
        target,
        removed_count,
        inserted_count,
    );
}

pub(crate) fn live_ranges_child_insertion(
    scope: &mut v8::PinScope<'_, '_>,
    parent: super::document_runtime::DomHandle,
    index: u32,
    inserted_child: super::document_runtime::DomHandle,
) {
    update_live_ranges_for_child_insertion(scope, parent, index, inserted_child);
}

pub(crate) fn live_ranges_child_removal(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &crate::dom::native::DomHost,
    parent: super::document_runtime::DomHandle,
    removed_child: super::document_runtime::DomHandle,
    index: u32,
    previous_sibling: Option<super::document_runtime::DomHandle>,
) {
    update_live_ranges_for_child_removal(
        scope,
        host_ptr,
        dom_host,
        parent,
        removed_child,
        index,
        previous_sibling,
    );
}

pub(crate) fn live_ranges_detached_child_insertion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    index: u32,
) {
    update_live_ranges_for_detached_child_insertion(scope, parent, index);
}

pub(crate) fn live_ranges_text_split(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    original: super::document_runtime::DomHandle,
    new_text: super::document_runtime::DomHandle,
    offset: u32,
) {
    update_live_ranges_for_text_split(scope, host_ptr, original, new_text, offset);
}

pub(crate) fn live_ranges_detached_text_split<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Object>,
    new_text: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    update_live_ranges_for_detached_text_split(scope, original, new_text, offset);
}

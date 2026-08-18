pub(crate) mod abort;
mod active_child_window;
pub(super) mod bindings;
mod bridge;
mod child_window_surface;
mod collections;
mod context_host;
pub(crate) use context_host::{
    JsContextHost, JsContextHostPageTaskCapabilities, PendingScrollObservableEffects,
    PostParseAutofocusAdmission, ServiceWorkerWindowOwner,
};
pub(crate) mod document;
pub(crate) mod element;
mod helpers;
mod history_queue;
pub(super) mod identity;
mod node;
pub(crate) mod pointer_lock;
mod traversal;
mod window;

use super::{
    document_runtime::DomHandle,
    reflector::ReflectorId,
    util::{callback_arg_string, v8_string},
};

use bindings::NativeBridgeBindings;
pub(crate) use history_queue::{
    NavigationAttemptId, PendingChildCrossDocumentTraversal, PendingHistoryTraversal,
    PendingHistoryTraversalAction, PendingNavigationApiTaskAction, PendingNavigationFinishedResult,
    PendingNavigationResult,
};
use identity::{BridgeHandle, BridgeIdentityStore, DomTokenListKind};
use identity::{CollectionKind, LiveCollectionDescriptor, LiveCollectionQueryKind};
pub(crate) use identity::{
    ComputedStyleDescriptor, ComputedStylePseudoKey, ComputedStyleTargetKey,
    clear_context_wrapper_cache_for_teardown,
};

pub(crate) use active_child_window::{
    active_child_window_handle, child_window_handle_from_marker_data,
    defer_active_child_window_restore, enter_active_child_window_scope,
    entered_child_window_handle, restore_active_child_window_scope,
    restore_deferred_active_child_window_scope_if_present,
};
pub(crate) use child_window_surface::CALLBACK_ERROR_WINDOW_HANDLE_SLOT;
pub(crate) use collections::{
    blob_parts_platform_collection_kind, install_collection_template_bindings,
};
pub(crate) use context_host::{
    DetachedChildBrowsingContextDocumentSnapshot, ImageDecodeRequestId,
    RuntimeObservableContextToken, cross_origin_lightweight_popup_id,
    current_runtime_observable_context_token, defer_active_lightweight_popup_restore,
    enter_active_lightweight_popup_scope, enter_top_level_lightweight_popup_scope,
    install_child_window_proxy_access_check_handlers,
    install_runtime_observable_context_token_for_context, is_cross_origin_top_window_proxy,
    lightweight_popup_id_from_window, restore_active_lightweight_popup_scope,
    restore_deferred_active_lightweight_popup_scope_if_present,
    throw_cross_origin_location_security_error, throw_cross_origin_type_error,
};

pub(crate) const ACTIVE_CHILD_WINDOW_HANDLE_SLOT: &str = "__moliActiveChildWindowHandle";
pub(crate) const ENTERED_CHILD_WINDOW_HANDLE_SLOT: &str = "__moliEnteredChildWindowHandle";

pub(crate) use bridge::wrapped_handle_value;
pub(super) use bridge::*;
pub(super) use context_host::*;
pub(crate) use document::install_document_template_bindings;
pub(crate) use element::{
    compute_mock_intersection_client_rect, compute_mock_intersection_scrollport_client_rect,
};
pub(super) use helpers::*;
pub(crate) use node::{
    current_or_live_delegate_node_arg_handle, node_or_foreign_arg_handle_allow_detached,
    node_runtime_and_handle_from_object, node_runtime_and_handle_from_object_or_detached,
    object_is_node_wrapper_or_detached, validate_pre_insert_handles,
};
pub(crate) use node::{install_character_data_template_bindings, install_node_template_bindings};

pub(crate) fn object_is_native_event_target_wrapper_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if node::receiver_has_detached_state(scope, object) {
        return true;
    }
    bridge_handle_from_object(scope, object)
        .is_ok_and(|(_, handle)| matches!(handle, BridgeHandle::Node(_) | BridgeHandle::Window))
}
pub(crate) use traversal::install_traversal_template_bindings;

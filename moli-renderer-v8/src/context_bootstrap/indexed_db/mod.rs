use super::{
    array_contains_strict, array_push_value, context_host_ptr_from_global_bridge,
    define_non_enumerable_value_property as define_public_non_enumerable_value_property,
    global_constructor_prototype, object_bool_property as public_object_bool_property,
    object_number_property as public_object_number_property,
    object_property_as_object as public_object_property_as_object,
    object_string_property as public_object_string_property, object_string_property_defined,
    throw_type_error, v8_string, v8str,
};
use crate::util::{new_null_prototype_object, private_key, set_private_value};

mod backend;
mod core;
mod cursor;
mod database;
mod event_target;
mod install;
mod operation_state;
mod runtime;
mod slots;
mod state;
mod storage_bucket;
mod stores;
mod tasks;
mod typed_state;
mod types;

use self::backend::*;
use self::core::*;
use self::cursor::*;
use self::database::*;
use self::event_target::*;
use self::operation_state::*;
use self::runtime::*;
use self::slots::*;
use self::storage_bucket::{
    storage_bucket_quota_check_for_object_store, storage_bucket_quota_check_for_transaction,
    validate_storage_bucket_scope,
};
use self::stores::*;
use self::tasks::*;
use self::typed_state::*;
use self::types::*;

pub(in crate::context_bootstrap) use self::event_target::idb_version_change_event_constructor_callback;

pub(crate) use self::runtime::{
    bind_indexed_db_factory_to_window_execution_context, indexed_db_has_pending_tasks,
    materialized_indexed_db_factory_for_window, scoped_indexed_db_factory,
    set_worker_indexed_db_task_wake_for_context,
};
pub(crate) use self::tasks::{
    discard_indexed_db_task_by_id, flush_indexed_db_task_by_id, flush_next_indexed_db_task,
};
pub(crate) use self::typed_state::IndexedDbTaskId;
pub(crate) use self::typed_state::deactivate_indexed_db_transaction_after_microtask_checkpoint;
pub(in crate::context_bootstrap::indexed_db) use self::typed_state::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;

pub(crate) fn flush_blocked_indexed_db_requests(scope: &mut v8::PinScope<'_, '_>) {
    flush_drain_blocked_open_requests_task(scope);
}

pub(in crate::context_bootstrap) use self::core::indexed_db_usage_bytes_for_storage_key;
pub(crate) use self::core::set_indexed_db_manager_for_context;
#[cfg(test)]
pub(crate) use self::core::{
    indexed_db_manager_context_slot_present_for_test,
    indexed_db_manager_isolate_slot_present_for_test,
};
pub(crate) use self::install::install_worker_indexed_db_runtime_state;
pub(super) use self::install::{
    ensure_indexed_db_runtime_state, install_indexed_db_template_bindings, window_indexed_db_getter,
};
pub use self::state::{
    SharedIndexedDbManager, WeakIndexedDbManager, clear_indexed_db_origin,
    clear_indexed_db_origins_with_prefix, downgrade_indexed_db_manager,
    indexed_db_origin_usage_bytes, indexed_db_origins_with_prefix_usage_bytes,
    new_indexed_db_manager,
};
pub(in crate::context_bootstrap) use self::storage_bucket::scoped_storage_bucket_indexed_db_factory;
pub use moli_indexeddb::{
    IndexedDbQuotaCheck, Key, ObjectStoreOptions, OpenOptions, TransactionMode,
};

fn is_indexed_db_private_slot(slot: &str) -> bool {
    slot.starts_with("moli.IndexedDb.")
}

fn indexed_db_private_slot_fallback_allowed(slot: &str) -> bool {
    if slot.starts_with("moli.IndexedDb.runtime.") {
        return true;
    }
    matches!(
        slot,
        INDEXED_DB_TYPED_STATE_ID_SLOT
            | INDEXED_DB_TYPED_TASK_ID_SLOT
            | INDEXED_DB_EVENT_LISTENERS_SLOT
    )
}

fn define_non_enumerable_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    debug_assert!(is_indexed_db_private_slot(key));
    if set_indexed_db_typed_slot_value(scope, object, key, value) {
        return;
    }
    if !indexed_db_private_slot_fallback_allowed(key) {
        return;
    }
    set_private_value(scope, object, key, value);
}

fn define_non_enumerable_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: bool,
) {
    debug_assert!(is_indexed_db_private_slot(key));
    let value = v8::Boolean::new(scope, value);
    if set_indexed_db_typed_slot_value(scope, object, key, value.into()) {
        return;
    }
    if !indexed_db_private_slot_fallback_allowed(key) {
        return;
    }
    set_private_value(scope, object, key, value.into());
}

fn set_indexed_db_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    debug_assert!(is_indexed_db_private_slot(key));
    if set_indexed_db_typed_slot_value(scope, object, key, value) {
        return;
    }
    if !indexed_db_private_slot_fallback_allowed(key) {
        return;
    }
    set_private_value(scope, object, key, value);
}

fn set_indexed_db_internal_object_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(key) = v8_string(scope, key) else {
        return;
    };
    let _ = object.define_own_property(scope, key.into(), value, v8::PropertyAttribute::NONE);
}

fn object_own_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8_string(scope, key)?;
    if !object.has_own_property(scope, key.into()).unwrap_or(false) {
        return None;
    }
    object.get(scope, key.into())
}

fn object_own_property_as_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    object_own_value(scope, object, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn indexed_db_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = private_key(scope, key)?;
    let value = object.get_private(scope, key)?;
    if value.is_undefined() {
        None
    } else {
        Some(value)
    }
}

fn object_hidden_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    debug_assert!(is_indexed_db_private_slot(key));
    if let Some(value) = indexed_db_typed_slot_value(scope, object, key) {
        return Some(value);
    }
    if !indexed_db_private_slot_fallback_allowed(key) {
        return None;
    }
    indexed_db_private_value(scope, object, key)
}

fn object_property_as_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_indexed_db_private_slot(key) {
        object_hidden_value(scope, object, key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    } else {
        public_object_property_as_object(scope, object, key)
    }
}

fn indexed_db_request_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    object_hidden_value(scope, request, INDEXED_DB_REQUEST_TRANSACTION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_indexed_db_request_surface_value(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    slot: &'static str,
    property: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(relevant_context) = request.get_creation_context(scope) else {
        return;
    };
    set_indexed_db_slot_value(scope, request, slot, value);
    if relevant_context == scope.get_current_context() {
        let _ = request.set(scope, v8str(scope, property).into(), value);
        return;
    }

    let request = v8::Global::new(scope, request);
    let value = v8::Global::new(scope, value);
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let request = v8::Local::new(target_scope, &request);
    let value = v8::Local::new(target_scope, &value);
    let _ = request.set(target_scope, v8str(target_scope, property).into(), value);
}

fn object_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<bool> {
    if is_indexed_db_private_slot(key) {
        object_hidden_value(scope, object, key).map(|value| value.boolean_value(scope))
    } else {
        public_object_bool_property(scope, object, key)
    }
}

fn object_number_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<f64> {
    if is_indexed_db_private_slot(key) {
        object_hidden_value(scope, object, key)?.number_value(scope)
    } else {
        public_object_number_property(scope, object, key)
    }
}

fn object_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<String> {
    if is_indexed_db_private_slot(key) {
        object_hidden_value(scope, object, key)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
    } else {
        public_object_string_property(scope, object, key)
    }
}

use super::media_queries::simple_object_event_set_handler;
use super::shared::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
use crate::{
    util::context_host_ptr_from_global_bridge,
    util::{
        callback_data_index_value, callback_data_item, get_private_value, set_private_value,
        throw_range_error, throw_type_error,
    },
    worker::{
        WorkerGlobalKind, WorkerNetworkPolicy, WorkerSpawnOptions, spawn_worker_with_options,
        worker_secure_context_for_script_url,
    },
};

#[cfg(test)]
use crate::worker::WorkerHandle;

#[cfg(test)]
const WORKER_HANDLE_SLOT: &str = "__moliWorkerHandle";
const WORKER_ID_SLOT: &str = "__moliWorkerId";
const WORKER_ONMESSAGE_SLOT: &str = "__moliWorkerOnMessage";
const WORKER_ONMESSAGEERROR_SLOT: &str = "__moliWorkerOnMessageError";
const WORKER_ONERROR_SLOT: &str = "__moliWorkerOnError";

mod constructor;
mod dispatch;
mod methods;

pub(in crate::context_bootstrap) use constructor::{
    WorkerOptionsMembers, document_query_encoding_override, is_cross_origin_http_worker_script,
    materialize_worker_script_source, resolve_worker_script_url, throw_worker_dom_exception,
    trusted_worker_script_url_string_or_throw, worker_constructor_base_url,
    worker_script_resource_url, worker_script_scheme_can_load,
};

pub(super) use constructor::worker_constructor_callback;
pub(crate) use dispatch::{
    dispatch_worker_error_event_with_kind, dispatch_worker_event,
    flush_pending_worker_messages_for_listener, worker_has_message_delivery_listener,
};
pub(super) use methods::{worker_post_message_callback, worker_terminate_callback};

/// Browser tasks use the Worker's creation realm, including when it is passed
/// from another Window. Worker IDs are local to that realm's native host.
pub(in crate::context_bootstrap) fn queue_worker_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    task: crate::worker::WorkerMessage,
) {
    let Some(worker) = moli_webapi_declare::web_api_object_target(scope, worker) else {
        return;
    };
    let Some(context) = worker.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(worker_id) = constructor::worker_id(scope, worker) else {
        #[cfg(test)]
        if let Some(handle) = constructor::get_worker_handle(scope, worker) {
            unsafe { &*handle }.queue_task(task);
        }
        return;
    };
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.queue_worker_task(worker_id, task);
    } else {
        let _ = crate::worker::queue_nested_worker_task(scope, worker_id, task);
    }
}

#[cfg(test)]
mod tests;

use super::media_queries::{
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback, simple_object_event_set_ordered_handler,
};
use super::shared::{SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT};
use crate::{
    util::context_host_ptr_from_global_bridge,
    util::{
        callback_data_index_value, callback_data_item, get_private_value, set_private_value,
        throw_range_error, throw_type_error,
    },
    worker::{
        WorkerGlobalKind, WorkerNetworkPolicy, WorkerSpawnOptions,
        worker_secure_context_for_script_url,
    },
};

#[cfg(test)]
use crate::worker::{WorkerHandle, spawn_worker_with_options};
#[cfg(test)]
const WORKER_HANDLE_SLOT: &str = "__moliWorkerHandle";
const WORKER_ID_SLOT: &str = "__moliWorkerId";
const WORKER_LISTENERS_SLOT: &str = "__moliWorkerListeners";
const WORKER_ONMESSAGE_SLOT: &str = "__moliWorkerOnMessage";
const WORKER_ONMESSAGEERROR_SLOT: &str = "__moliWorkerOnMessageError";
const WORKER_ONERROR_SLOT: &str = "__moliWorkerOnError";

mod constructor;
mod dispatch;
mod methods;

pub(in crate::context_bootstrap) use constructor::{
    document_query_encoding_override, is_cross_origin_http_worker_script,
    materialize_worker_script_source, resolve_worker_script_url, throw_worker_dom_exception,
    trusted_worker_script_url_string_or_throw, worker_constructor_base_url,
    worker_script_resource_url, worker_script_scheme_can_load,
};

pub(super) use constructor::worker_constructor_callback;
pub(crate) use dispatch::{
    dispatch_worker_error_event_with_error, dispatch_worker_error_event_with_kind,
    dispatch_worker_event, flush_pending_worker_messages_for_listener,
    worker_has_message_delivery_listener,
};
pub(super) use methods::{worker_post_message_callback, worker_terminate_callback};

#[cfg(test)]
mod tests;

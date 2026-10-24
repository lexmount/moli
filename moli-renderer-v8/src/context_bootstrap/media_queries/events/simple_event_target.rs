use super::*;

mod callbacks;
mod dispatch;
mod handlers;
mod listeners;
mod state;

pub(crate) use callbacks::{
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback,
};
pub(in crate::context_bootstrap::media_queries::events::simple_event_target) use dispatch::simple_object_event_target_dispatch;
pub(crate) use dispatch::{
    dispatch_simple_event_target_event, dispatch_simple_event_target_event_collecting_errors,
    dispatch_simple_event_target_event_with_checkpoint,
    dispatch_simple_event_target_event_with_original_target, invoke_simple_event_target_listeners,
};
pub(crate) use handlers::{
    simple_event_target_handler_getter_callback, simple_event_target_handler_setter_callback,
};
use listeners::SimpleObjectEventListenerSnapshot;
pub(crate) use listeners::simple_object_event_target_register_webidl_listener;
pub(crate) use listeners::{
    SimpleObjectEventListenerInspectorSnapshot, clear_simple_event_target_listeners,
    simple_event_target_inspector_listener_snapshots,
    simple_object_event_activate_uncompiled_handler, simple_object_event_listeners_snapshot,
    simple_object_event_remove_listener_value_for_type, simple_object_event_set_handler,
    simple_object_has_event_listeners,
};
use listeners::{
    simple_object_event_listeners_snapshot_in_registry,
    simple_object_event_set_handler_in_registry,
    simple_object_event_target_add_listener_for_target,
    simple_object_event_target_remove_listener_in_registry,
};
use state::{
    ResolvedSimpleEventTarget, SimpleEventListenerRegistry, resolve_simple_event_target,
    simple_object_event_listener_registry,
};
pub(crate) use state::{initialize_simple_event_target, is_simple_event_target};

#[cfg(test)]
pub(crate) use handlers::install_simple_event_target_handlers;

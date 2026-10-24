use super::{SIMPLE_EVENT_TARGET_LISTENERS_SLOT, object_string_property_defined, v8str};
const SIMPLE_EVENT_TARGET_HANDLER_SLOT_FIELD: &str = "handlerSlot";

mod media_query_list;
mod simple_event_target;

pub(in crate::context_bootstrap) use media_query_list::{
    dispatch_media_query_list_event, media_query_list_add_listener_callback,
    media_query_list_remove_listener_callback,
};
pub(crate) use simple_event_target::{
    SimpleObjectEventListenerInspectorSnapshot, clear_simple_event_target_listeners,
    dispatch_simple_event_target_event, dispatch_simple_event_target_event_collecting_errors,
    dispatch_simple_event_target_event_with_checkpoint,
    dispatch_simple_event_target_event_with_original_target, initialize_simple_event_target,
    invoke_simple_event_target_listeners, is_simple_event_target,
    dispatch_simple_event_target_event_with_ancestors,
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_handler_getter_callback, simple_event_target_handler_setter_callback,
    simple_event_target_inspector_listener_snapshots,
    simple_event_target_remove_event_listener_callback,
    simple_object_event_activate_uncompiled_handler, simple_object_event_listeners_snapshot,
    simple_object_event_remove_listener_value_for_type, simple_object_event_set_handler,
    simple_object_event_target_register_webidl_listener, simple_object_has_event_listeners,
};

#[cfg(test)]
pub(crate) use simple_event_target::install_simple_event_target_handlers;

mod evaluation;
mod event;
pub(in crate::context_bootstrap) use event::build_media_query_list_event_template;
mod events;

pub(crate) use self::evaluation::dispatch_media_query_list_change_events;
pub(crate) use self::evaluation::evaluate_match_media_query_list_with_viewport;
pub(super) use self::evaluation::{
    install_media_query_list_template_bindings, window_match_media_callback,
};
pub(in crate::context_bootstrap) use self::events::dispatch_media_query_list_event;
pub(crate) use self::events::{
    SimpleObjectEventListenerInspectorSnapshot, clear_simple_event_target_listeners,
    dispatch_simple_event_target_event, dispatch_simple_event_target_event_collecting_errors,
    dispatch_simple_event_target_event_with_checkpoint,
    dispatch_simple_event_target_event_with_original_target, initialize_simple_event_target,
    invoke_simple_event_target_listeners, is_simple_event_target,
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_handler_getter_callback, simple_event_target_handler_setter_callback,
    simple_event_target_inspector_listener_snapshots,
    simple_event_target_remove_event_listener_callback,
    simple_object_event_activate_uncompiled_handler, simple_object_event_listeners_snapshot,
    simple_object_event_set_handler, simple_object_event_target_register_webidl_listener,
    simple_object_has_event_listeners,
};
pub(super) use self::events::{
    media_query_list_add_listener_callback, media_query_list_remove_listener_callback,
};

use super::{SIMPLE_EVENT_TARGET_LISTENERS_SLOT, object_string_property_defined, v8str};

#[cfg(test)]
pub(crate) use self::events::install_simple_event_target_handlers;

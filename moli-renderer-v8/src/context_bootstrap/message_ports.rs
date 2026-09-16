use super::super::exception_reporting::V8ExceptionReport;
use super::*;

mod constructors;
mod delivery;
mod methods;
mod scheduling;
mod state;

pub(super) use constructors::{
    message_channel_constructor_callback, message_port_constructor_callback,
};
pub(in crate::context_bootstrap) use delivery::schedule_message_port_delivery;
pub(crate) use delivery::{
    MessagePortDeliveryRunResult, dispatch_message_port_events_for_port_collecting_errors,
    dispatch_one_authorized_message_port_event,
};
pub(super) use methods::{
    message_port_close_callback, message_port_post_message_callback, message_port_start_callback,
};
pub(super) use scheduling::{schedule_host_callback, schedule_scope_callback};
pub(in crate::context_bootstrap) use state::install_message_port_template_bindings;
pub(crate) use state::{
    MessagePortRealmBinding, detach_message_port_owner_for_transfer,
    detach_transferred_message_port, ensure_message_port_wrapper_for_id,
    ensure_message_port_wrapper_for_id_in_realm, message_port_id_from_object,
};
pub(in crate::context_bootstrap) use state::{
    close_message_port_object, current_message_port_registry, discard_message_port_channel,
    set_internal_message_port_handlers,
};
pub(in crate::context_bootstrap::message_ports) use state::{
    forget_message_port_wrapper, message_port_is_closed, message_port_is_started,
    new_message_port_object, set_message_port_peer, set_message_port_started,
};

const MESSAGE_PORT_EVENT_LISTENERS_SLOT: &str = "__moliMessagePortEventListeners";

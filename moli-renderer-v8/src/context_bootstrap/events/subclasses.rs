use super::base::{define_event_property, event_type_argument, initialize_event_object};
use super::init::{
    init_bool_property, init_number_property, init_string_property, init_value_property,
    init_window_view_property, read_event_init,
};
use super::kind::EventSubclassKind;
use super::*;

mod basic;
mod constructor;
mod data;
mod keyboard;
mod navigation_init;
mod pointer;

pub(in crate::context_bootstrap::events) use basic::initialize_text_event;
pub(in crate::context_bootstrap) use constructor::build_event_subclass_template;
pub(in crate::context_bootstrap) use data::{
    navigation_precommit_controller_for_event, run_navigate_event_precommit_handlers,
};
pub(in crate::context_bootstrap) use pointer::{
    pointer_event_get_coalesced_events_callback, pointer_event_get_predicted_events_callback,
};

pub(in crate::context_bootstrap::events) use data::initialize_navigate_event_methods;

use super::location_navigation::{
    LocationNavigationKind, navigate_location_object,
    navigate_location_object_with_child_navigate_event,
};
use super::navigation_callbacks::{document_location_getter, document_location_setter};
use super::*;

mod helpers;
mod install;
mod methods;
mod navigation;
mod slots;
mod surface;

pub(super) use install::{
    build_location_constructor_template, build_location_runtime_object,
    install_location_runtime_state, location_belongs_to_current_local_window,
    location_owner_has_current_realm,
};
pub(super) use navigation::{
    is_same_document_fragment_navigation, resolve_location_navigation_target,
    urls_refer_to_same_document,
};
pub(super) use slots::{location_href_slot, sync_location_object};
pub(crate) use surface::sync_global_location_runtime_state;
pub(crate) use surface::{
    install_constructed_document_location_runtime_state,
    sync_document_location_runtime_state_from_window,
    sync_window_location_history_navigation_runtime_surface, sync_window_location_runtime_state,
};
pub(in crate::context_bootstrap) use surface::{window_location_setter, window_navigation_setter};

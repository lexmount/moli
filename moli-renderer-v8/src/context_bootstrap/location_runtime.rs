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
    build_location_runtime_object, ensure_location_constructor_runtime_state,
    install_location_runtime_state,
};
pub(super) use navigation::{is_same_document_fragment_navigation, urls_refer_to_same_document};
pub(crate) use navigation::{
    resolve_location_navigation_target, resolve_location_navigation_target_against_entered_base,
};
pub(super) use slots::{location_href_slot, sync_location_object};
pub(crate) use surface::sync_global_location_runtime_state;
pub(crate) use surface::{
    navigate_top_level_window_location_from_cross_origin,
    sync_document_location_runtime_state_from_window,
    sync_window_location_history_navigation_runtime_surface,
};
pub(in crate::context_bootstrap) use surface::{window_location_setter, window_navigation_setter};

use super::location_navigation::LocationNavigationKind;
use super::location_runtime::sync_location_object;
use super::navigation_activation::{
    bind_navigation_entry_runtime_owner, install_navigation_activation_runtime_state,
    set_navigation_current_entry,
};
use super::navigation_entry::{
    copy_navigation_entry_document_id, create_navigation_entry, history_entries, history_index,
    navigation_current_entry, navigation_current_entry_index, navigation_entry_key_value,
    navigation_entry_url_value, new_navigation_entry_id, new_navigation_entry_key,
    set_history_entries, set_history_index, set_history_state, set_navigation_entry_document_id,
    set_navigation_entry_joint_top_index, stringify_history_state,
    sync_navigation_current_entry_from_history_entry,
};
use super::navigation_entry_state::{
    clone_history_entry_state, clone_navigation_entry_state, set_navigation_entry_state,
};
use super::navigation_events::{
    dispatch_navigation_currententrychange, dispatch_navigation_entry_dispose,
    refresh_navigation_destination_indexes,
};
use super::navigation_projection::{
    build_visible_navigation_entries_array, set_history_length_at_least_visible_entries,
    set_history_length_from_visible_entries,
};
use super::navigation_serialize::{
    serialize_history_entries, serialize_navigation_entry_object,
    sync_child_navigation_entry_seed_from_owner,
    sync_child_pending_navigation_entry_seed_from_owner,
};
use super::navigation_window::{
    navigation_document_is_initial_empty, runtime_top_window_owner, runtime_window_is_global,
    window_history_for_holder, window_location_for_holder, window_navigation_for_holder,
};
use super::*;
use crate::native_bridge::NavigationActivationSeed;

mod document_front;
mod local;
mod same_document;

pub(super) use document_front::sync_local_document_front_from_window;
pub(crate) use local::apply_local_window_location_navigation;
pub(super) use same_document::{
    apply_navigation_navigate_same_document, update_navigation_current_entry_for_same_document,
};
